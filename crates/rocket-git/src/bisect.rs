//! Manual Git bisect with a durable return checkout and non-forced transitions.
//!
//! Git's no-checkout engine records decisions independently of file changes.
//! Interrupted transitions can therefore be inspected/resumed without submitting
//! a decision twice. Automated tests require a separate explicit command review.
//! Never replay a log or reset --hard.
use crate::{model::bytes_to_os, parse, runner::args, *};
use std::{fs, io::Write, path::PathBuf};

pub(crate) const JOURNAL: &str = "rocket-bisect";
const LIMIT: usize = 10_000;
mod automatic;
pub use automatic::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectAction {
    Start,
    Good,
    Bad,
    Skip,
    Resume,
    Restore,
}
impl BisectAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "bisect-start",
            Self::Good => "bisect-good",
            Self::Bad => "bisect-bad",
            Self::Skip => "bisect-skip",
            Self::Resume => "bisect-resume",
            Self::Restore => "bisect-restore",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectFailure {
    ActiveOperation,
    DirtyWorktree,
    UnsupportedIndex,
    UnsupportedHistory,
    InvalidRange,
    RevisionUnavailable,
    RangeTooLarge,
    StalePreview,
    IndexLocked,
    ForeignSession,
    JournalChanged,
    OriginalBranchMoved,
    CheckoutBlocked,
    UnrecordedCheckout,
    NotReady,
    CommandFailed,
}
impl BisectFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::ActiveOperation => "bisect_active_operation",
            Self::DirtyWorktree => "bisect_dirty_worktree",
            Self::UnsupportedIndex => "bisect_unsupported_index",
            Self::UnsupportedHistory => "bisect_unsupported_history",
            Self::InvalidRange => "bisect_invalid_range",
            Self::RevisionUnavailable => "bisect_revision_unavailable",
            Self::RangeTooLarge => "bisect_range_too_large",
            Self::StalePreview => "bisect_stale_preview",
            Self::IndexLocked => "bisect_index_locked",
            Self::ForeignSession => "bisect_foreign_session",
            Self::JournalChanged => "bisect_journal_changed",
            Self::OriginalBranchMoved => "bisect_original_branch_moved",
            Self::CheckoutBlocked => "bisect_checkout_blocked",
            Self::UnrecordedCheckout => "bisect_unrecorded_checkout",
            Self::NotReady => "bisect_not_ready",
            Self::CommandFailed => "bisect_command_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::ActiveOperation => {
                "Finish the active Git operation before starting another one. For an active Rocket bisect, open Bisect and review Restore original."
            }
            Self::DirtyWorktree => {
                "Commit or stash tracked edits before starting. If you edited files during bisect, preserve them with Git or copy them aside before continuing. Rocket has not discarded or automatically stashed your work."
            }
            Self::UnsupportedIndex => {
                "The index has hidden entries or submodules. Review assume-unchanged, skip-worktree and submodule state with Git first; Rocket leaves them unchanged."
            }
            Self::UnsupportedHistory => {
                "Bisect requires complete local history without replacement refs, grafts or submodules in its range. Use Git for those advanced cases."
            }
            Self::InvalidRange => {
                "Choose two different local commits: the known good commit must be an ancestor of the known bad commit."
            }
            Self::RangeTooLarge => {
                "This range exceeds the 10,000-commit review limit. Choose a more recent known good commit."
            }
            Self::RevisionUnavailable => {
                "A selected revision could not be resolved to a local commit. Check the name or commit ID and fetch separately if needed, then review the range again."
            }
            Self::StalePreview => {
                "The checkout, index, selected revision or bisect state changed after review. Refresh and review again; the requested action was not applied."
            }
            Self::IndexLocked => {
                "Git's index is locked. Let other Git work finish, then refresh. Rocket never deletes an existing lock."
            }
            Self::ForeignSession => {
                "This bisect was started outside Rocket, or its metadata no longer matches Rocket's saved session. Inspect it with git bisect log and finish it with Git; Rocket will not reset an unknown session."
            }
            Self::JournalChanged => {
                "The saved bisect recovery record is missing, changed or invalid. Keep it and inspect the repository with Git before further changes."
            }
            Self::OriginalBranchMoved => {
                "The original branch moved, was renamed or was deleted during bisect. Rocket refuses to reset it. The saved original commit remains available in the session's recovery ref; inspect and recover with Git."
            }
            Self::CheckoutBlocked => {
                "Git refused the checkout, possibly because an untracked or ignored file would be overwritten, or the branch is in another worktree. Your decision may already be recorded. Protect those files, then Refresh and Resume checkout or Restore."
            }
            Self::UnrecordedCheckout => {
                "HEAD moved outside this search, possibly to a new detached commit. Retain that work on a branch with Git, then return to the original checkout or a recorded bisect revision before restoring. Rocket will not abandon an unrecorded checkout."
            }
            Self::NotReady => {
                "This revision is not ready for another decision. Refresh and Resume checkout, or Restore the original checkout. Found and inconclusive results cannot be marked again."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Refresh the bisect session before retrying. Cancellation is not rollback; a decision or checkout may already have completed."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    original: Head,
    good: ObjectId,
    bad: ObjectId,
    recovery: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectPhase {
    Testing,
    CheckoutPending,
    Found,
    Inconclusive,
    RestoreOnly,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BisectDecision {
    pub action: BisectAction,
    pub commit: ObjectId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BisectSession {
    pub original: Head,
    pub good: ObjectId,
    pub bad: ObjectId,
    pub recovery_reference: String,
    pub current: Head,
    pub target: Option<ObjectId>,
    pub candidates: Vec<ObjectId>,
    pub decisions: Vec<BisectDecision>,
    pub phase: BisectPhase,
}
impl BisectSession {
    pub fn can_mark(&self) -> bool {
        self.phase == BisectPhase::Testing
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    head: Head,
    index: Vec<u8>,
    metadata: Vec<(String, Vec<u8>)>,
    refs: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct BisectPreview {
    git_dir: PathBuf,
    common_dir: PathBuf,
    worktree: Option<PathBuf>,
    context: Context,
    saved: Saved,
    action: BisectAction,
    good_input: String,
    bad_input: String,
    session: Option<BisectSession>,
    candidates: usize,
}
impl BisectPreview {
    pub fn action(&self) -> BisectAction {
        self.action
    }
    pub fn original(&self) -> &Head {
        &self.saved.original
    }
    pub fn good(&self) -> &ObjectId {
        &self.saved.good
    }
    pub fn bad(&self) -> &ObjectId {
        &self.saved.bad
    }
    pub fn session(&self) -> Option<&BisectSession> {
        self.session.as_ref()
    }
    pub fn candidates(&self) -> usize {
        self.candidates
    }
    pub fn good_input(&self) -> &str {
        &self.good_input
    }
    pub fn bad_input(&self) -> &str {
        &self.bad_input
    }
}
#[derive(Debug)]
#[must_use]
pub struct BisectWriteReport {
    pub result: Result<WriteOutput>,
    pub status: Result<Status>,
    pub session: Result<Option<BisectSession>>,
    pub state: Result<OperationState>,
    pub failure: Option<BisectFailure>,
}

impl Repository {
    /// Cheap worktree-local marker observation for write guards. Ownership and
    /// complete state are checked separately by `bisect_session`.
    pub fn is_bisecting(&self, cancel: &CancellationToken) -> Result<bool> {
        cancel.check()?;
        if self.info().worktree.is_none() {
            return Ok(false);
        }
        for name in ["BISECT_START", JOURNAL] {
            match fs::symlink_metadata(self.info().git_dir.join(name)) {
                Ok(_) => return Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("inspect bisect marker", e)),
            }
        }
        cancel.check()?;
        Ok(false)
    }
    pub fn bisect_session(&self, cancel: &CancellationToken) -> Result<Option<BisectSession>> {
        self.require_worktree(cancel)?;
        let context = self.bisect_context(false, cancel)?;
        let saved = self.bisect_saved(&context, cancel)?;
        let Some(saved) = saved else { return Ok(None) };
        self.bisect_session_from(&context, &saved, cancel).map(Some)
    }

    pub fn preview_bisect(
        &self,
        action: BisectAction,
        good: &str,
        bad: &str,
        cancel: &CancellationToken,
    ) -> Result<BisectPreview> {
        self.require_worktree(cancel)?;
        self.runner.step("bisect-preview", "inspect_session", || {
            let context = self.bisect_context(true, cancel)?;
            let existing = self.bisect_saved(&context, cancel)?;
            let (saved, session, candidates) = if action == BisectAction::Start {
                if existing.is_some() || has(&context, JOURNAL) || has(&context, "BISECT_START") {
                    return fail(BisectFailure::ActiveOperation);
                }
                self.bisect_supported(cancel)?;
                let first = self.bisect_revision(good, cancel)?;
                let last = self.bisect_revision(bad, cancel)?;
                let ancestor = self.runner.run(
                    self.cwd(),
                    &args(&[
                        "--no-replace-objects",
                        "merge-base",
                        "--is-ancestor",
                        first.as_str(),
                        last.as_str(),
                    ]),
                    Vec::new(),
                    cancel,
                )?;
                if first == last || ancestor.status.code() == Some(1) {
                    return fail(BisectFailure::InvalidRange);
                }
                ancestor.success("bisect-preview")?;
                let ids = self.bisect_candidates(&last, std::slice::from_ref(&first), cancel)?;
                // One bounded history read detects gitlink changes throughout
                // the range (including merge parents), avoiding N Git processes.
                let tree = self.bisect_read(
                    &["ls-tree", "-r", "--format=%(objectmode)", first.as_str()],
                    cancel,
                )?;
                let changes = self.bisect_read(
                    &[
                        "-c",
                        "log.showSignature=false",
                        "log",
                        "--no-color",
                        "--no-notes",
                        "--raw",
                        "-z",
                        "-m",
                        "--no-renames",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--format=",
                        last.as_str(),
                        &format!("^{}", first),
                        "--",
                    ],
                    cancel,
                )?;
                if tree.split(|b| *b == b'\n').any(|m| m == b"160000")
                    || changes.split(|b| *b == 0).any(|r| {
                        let r = r.trim_ascii();
                        r.starts_with(b":160000 ")
                            || (r.first() == Some(&b':') && r.get(8..14) == Some(b"160000"))
                    })
                {
                    return fail(BisectFailure::UnsupportedHistory);
                }
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| Error::Bisect(BisectFailure::JournalChanged))?
                    .as_nanos();
                let saved = Saved {
                    original: context.head.clone(),
                    good: first,
                    bad: last,
                    recovery: format!("refs/rocket/bisect/{}-{stamp}", std::process::id()),
                };
                (saved, None, ids.len())
            } else {
                let saved = existing.ok_or(Error::Bisect(BisectFailure::NotReady))?;
                let session = self.bisect_session_from(&context, &saved, cancel)?;
                // A commit made while detached is new user work, not a tested
                // candidate. Require the user to retain it on a branch first.
                let mut allowed =
                    self.bisect_candidates(&saved.bad, std::slice::from_ref(&saved.good), cancel)?;
                allowed.push(saved.good.clone());
                allowed.push(head_id(&saved.original)?.clone());
                if context.head != saved.original
                    && (!matches!(context.head, Head::Detached { .. })
                        || !allowed.contains(head_id(&context.head)?))
                {
                    return fail(BisectFailure::UnrecordedCheckout);
                }
                match action {
                    BisectAction::Good | BisectAction::Bad | BisectAction::Skip
                        if !session.can_mark() =>
                    {
                        return fail(BisectFailure::NotReady);
                    }
                    BisectAction::Resume if session.phase != BisectPhase::CheckoutPending => {
                        return fail(BisectFailure::NotReady);
                    }
                    BisectAction::Restore => self.bisect_original(&saved, cancel)?,
                    _ => {}
                }
                let count = session.candidates.len();
                (saved, Some(session), count)
            };
            if self.bisect_context(true, cancel)? != context {
                return fail(BisectFailure::StalePreview);
            }
            Ok(BisectPreview {
                git_dir: self.info().git_dir.clone(),
                common_dir: self.info().common_dir.clone(),
                worktree: self.info().worktree.clone(),
                context,
                saved,
                action,
                good_input: good.into(),
                bad_input: bad.into(),
                session,
                candidates,
            })
        })
    }

    pub fn apply_bisect(
        &self,
        preview: &BisectPreview,
        cancel: &CancellationToken,
    ) -> Result<BisectWriteReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.apply_bisect_locked(preview, cancel)
    }

    // The automated loop holds the same cross-process write permit across a
    // command, its classification and the next checkout.
    fn apply_bisect_locked(
        &self,
        preview: &BisectPreview,
        cancel: &CancellationToken,
    ) -> Result<BisectWriteReport> {
        let label = preview.action.label();
        self.runner.step(label, "revalidate_bisect", || {
            if self.info().git_dir != preview.git_dir
                || self.info().common_dir != preview.common_dir
                || self.info().worktree != preview.worktree
                || self.bisect_context(true, cancel)? != preview.context
            {
                return fail(BisectFailure::StalePreview);
            }
            if preview.action == BisectAction::Start {
                if self.bisect_revision(&preview.good_input, cancel)? != preview.saved.good
                    || self.bisect_revision(&preview.bad_input, cancel)? != preview.saved.bad
                {
                    return fail(BisectFailure::StalePreview);
                }
                self.bisect_supported(cancel)?;
            } else if preview.action == BisectAction::Restore {
                self.bisect_original(&preview.saved, cancel)?;
            } else {
                self.bisect_supported(cancel)?;
            }
            cancel.check()
        })?;
        let result = self
            .runner
            .observe("write_result", label, || self.bisect_apply(preview, cancel));
        let fresh = CancellationToken::new();
        let status = self
            .runner
            .step(label, "refresh_status", || self.status(&fresh));
        let session = self.runner.step(label, "inspect_bisect_result", || {
            self.bisect_session(&fresh)
        });
        let state = self
            .runner
            .step(label, "refresh_operation", || self.operation_state(&fresh));
        let failure = match &result {
            Err(Error::Bisect(f)) => Some(*f),
            Err(_) => Some(BisectFailure::CommandFailed),
            _ => None,
        };
        Ok(BisectWriteReport {
            result,
            status,
            session,
            state,
            failure,
        })
    }

    fn bisect_apply(&self, p: &BisectPreview, cancel: &CancellationToken) -> Result<WriteOutput> {
        let label = p.action.label();
        let saved = &p.saved;
        if p.action == BisectAction::Start {
            self.runner.step(label, "save_return_checkout", || {
                let mut file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(self.info().git_dir.join(JOURNAL))
                    .map_err(|e| Error::io("save bisect recovery", e))?;
                file.write_all(&saved.bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|e| Error::io("save bisect recovery", e))?;
                cancel.check()?;
                self.bisect_write(
                    &[
                        "update-ref",
                        "--no-deref",
                        &saved.recovery,
                        head_id(&saved.original)?.as_str(),
                        "",
                    ],
                    label,
                    cancel,
                )?;
                Ok(())
            })?;
            self.runner.step(label, "record_bisect_decision", || {
                self.bisect_engine(
                    &[
                        "start",
                        "--no-checkout",
                        saved.bad.as_str(),
                        saved.good.as_str(),
                        "--",
                    ],
                    label,
                    cancel,
                )
            })?;
        } else if p.action == BisectAction::Restore {
            self.runner.step(label, "restore_checkout", || {
                self.bisect_checkout(&saved.original, label, cancel)
            })?;
            // Restoration must be verified before discarding any search state.
            self.bisect_original(saved, cancel)?;
            if self.head(cancel)? != saved.original {
                return fail(BisectFailure::OriginalBranchMoved);
            }
            self.runner.step(label, "clear_bisect_state", || {
                let context = self.bisect_context(true, cancel)?;
                if self.bisect_saved(&context, cancel)?.as_ref() != Some(saved) {
                    return fail(BisectFailure::JournalChanged);
                }
                if has(&context, "BISECT_START") {
                    // Explicit HEAD also makes cleanup safe if an external tool
                    // removed BISECT_HEAD: never guess the checkout destination.
                    self.bisect_engine(&["reset", "HEAD"], label, cancel)?;
                }
                cancel.check()?;
                // A CAS deletion cannot remove somebody else's replacement ref.
                let symbolic = self.runner.run(
                    self.cwd(),
                    &args(&["symbolic-ref", "--quiet", &saved.recovery]),
                    Vec::new(),
                    cancel,
                )?;
                if symbolic.status.code() != Some(1) {
                    return fail(BisectFailure::JournalChanged);
                }
                let reference = self.runner.run(
                    self.cwd(),
                    &args(&["rev-parse", "--verify", "--quiet", &saved.recovery]),
                    Vec::new(),
                    cancel,
                )?;
                let expected = match reference.status.code() {
                    Some(0)
                        if reference.stdout.trim_ascii()
                            == head_id(&saved.original)?.as_str().as_bytes() =>
                    {
                        head_id(&saved.original)?.as_str().to_owned()
                    }
                    // Setup may stop before ref creation, or cleanup after ref
                    // deletion. Verify absence atomically rather than deleting
                    // a replacement ref that appeared after this read.
                    Some(1) => "0".repeat(head_id(&saved.original)?.as_str().len()),
                    _ => return fail(BisectFailure::JournalChanged),
                };
                self.bisect_write(
                    &["update-ref", "--no-deref", "-d", &saved.recovery, &expected],
                    label,
                    cancel,
                )?;
                let path = self.info().git_dir.join(JOURNAL);
                if crate::local_files::read_config(&path)? != saved.bytes() {
                    return fail(BisectFailure::JournalChanged);
                }
                fs::remove_file(path).map_err(|e| Error::io("clear bisect recovery", e))?;
                Ok(())
            })?;
            return Ok(WriteOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        } else if matches!(
            p.action,
            BisectAction::Good | BisectAction::Bad | BisectAction::Skip
        ) {
            let term = match p.action {
                BisectAction::Good => "good",
                BisectAction::Bad => "bad",
                _ => "skip",
            };
            self.runner.step(label, "record_bisect_decision", || {
                self.bisect_engine(&[term, head_id(&p.context.head)?.as_str()], label, cancel)
            })?;
        }
        let observed = self
            .bisect_session(cancel)?
            .ok_or(Error::Bisect(BisectFailure::NotReady))?;
        if let Some(target) = observed.target {
            self.runner.step(label, "checkout_bisect_candidate", || {
                self.bisect_checkout(&Head::Detached { target }, label, cancel)
            })?;
        }
        Ok(WriteOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn bisect_context(&self, clean: bool, cancel: &CancellationToken) -> Result<Context> {
        // File-based operations, including malformed loose markers, fail closed.
        // Ref-backed operations are also inspected below through Git, so this
        // guard works with reftable and each linked worktree's own root refs.
        for name in [
            "MERGE_HEAD",
            "MERGE_AUTOSTASH",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
        ] {
            cancel.check()?;
            match fs::symlink_metadata(self.info().git_dir.join(name)) {
                Ok(_) => return fail(BisectFailure::ActiveOperation),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("inspect bisect operation marker", e)),
            }
        }
        let status = self.status(cancel)?;
        head_id(&status.head)?;
        if clean {
            if self
                .info()
                .git_dir
                .join("index.lock")
                .try_exists()
                .map_err(|e| Error::io("bisect index lock", e))?
            {
                return fail(BisectFailure::IndexLocked);
            }
            if status
                .entries
                .iter()
                .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            {
                return fail(BisectFailure::DirtyWorktree);
            }
        }
        let index = self.bisect_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if clean
            && index.split(|b| *b == 0).filter(|e| !e.is_empty()).any(|e| {
                e[0].is_ascii_lowercase() || e[0] == b'S' || e.get(2..8) == Some(b"160000")
            })
        {
            return fail(BisectFailure::UnsupportedIndex);
        }
        let mut metadata = Vec::new();
        for name in [
            JOURNAL,
            "BISECT_START",
            "BISECT_NAMES",
            "BISECT_TERMS",
            "BISECT_EXPECTED_REV",
            "BISECT_LOG",
        ] {
            cancel.check()?;
            let path = self.info().git_dir.join(name);
            match fs::symlink_metadata(&path) {
                Ok(_) => metadata.push((name.into(), crate::local_files::read_config(&path)?)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("inspect bisect metadata", e)),
            }
        }
        // One Git read observes exact operation refs, BISECT_HEAD and search
        // refs. Do not use rev-parse: it can resolve a similarly named branch.
        let observed_refs = self.bisect_read(
            &[
                "for-each-ref",
                "--include-root-refs",
                "--format=%(refname) %(objectname)",
                "MERGE_AUTOSTASH",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
                "BISECT_HEAD",
                "refs/bisect/",
            ],
            cancel,
        )?;
        let mut refs = Vec::new();
        for line in observed_refs
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
        {
            let Some(separator) = line.iter().position(|b| *b == b' ') else {
                return fail(BisectFailure::ForeignSession);
            };
            let (name, id) = (&line[..separator], &line[separator + 1..]);
            if name == b"BISECT_HEAD" {
                parse::oid(id, "bisect candidate")?;
                metadata.push(("BISECT_HEAD".into(), id.to_vec()));
            } else if name.starts_with(b"refs/bisect/") {
                refs.extend_from_slice(line);
                refs.push(b'\n');
            } else {
                return fail(BisectFailure::ActiveOperation);
            }
        }
        Ok(Context {
            head: status.head,
            index,
            metadata,
            refs,
        })
    }
    fn bisect_saved(&self, context: &Context, cancel: &CancellationToken) -> Result<Option<Saved>> {
        let bytes = meta(context, JOURNAL);
        if bytes.is_empty() {
            if has(context, "BISECT_START")
                || !context.refs.is_empty()
                || has(context, "BISECT_HEAD")
            {
                return fail(BisectFailure::ForeignSession);
            }
            return Ok(None);
        }
        let saved = Saved::parse(bytes)?;
        if let Head::Branch { name, .. } = &saved.original {
            self.branch_argument(name, cancel)?;
        }
        if has(context, "BISECT_START") {
            let original = match &saved.original {
                Head::Branch { name, .. } => name.as_slice(),
                _ => head_id(&saved.original)?.as_str().as_bytes(),
            };
            if meta(context, "BISECT_START").trim_ascii() != original
                || meta(context, "BISECT_TERMS").trim_ascii() != b"bad\ngood"
                || !meta(context, "BISECT_NAMES").trim_ascii().is_empty()
            {
                return fail(BisectFailure::ForeignSession);
            }
        } else if !context.refs.is_empty() || has(context, "BISECT_HEAD") {
            return fail(BisectFailure::ForeignSession);
        }
        Ok(Some(saved))
    }
    fn bisect_session_from(
        &self,
        context: &Context,
        saved: &Saved,
        cancel: &CancellationToken,
    ) -> Result<BisectSession> {
        let mut s = BisectSession {
            original: saved.original.clone(),
            good: saved.good.clone(),
            bad: saved.bad.clone(),
            recovery_reference: saved.recovery.clone(),
            current: context.head.clone(),
            target: None,
            candidates: Vec::new(),
            decisions: Vec::new(),
            phase: BisectPhase::RestoreOnly,
        };
        if !has(context, "BISECT_START") {
            return Ok(s);
        }
        let mut bad = None;
        let (mut good, mut skipped) = (Vec::new(), Vec::new());
        for line in context
            .refs
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
        {
            let mut parts = line.splitn(2, |b| *b == b' ');
            let name = parts.next().unwrap_or_default();
            let Some(id) = parts.next() else {
                return fail(BisectFailure::ForeignSession);
            };
            let id = parse::oid(id, "bisect refs")?;
            if name == b"refs/bisect/bad" {
                bad = Some(id);
            } else if name.strip_prefix(b"refs/bisect/good-") == Some(id.as_str().as_bytes()) {
                good.push(id);
            } else if name.strip_prefix(b"refs/bisect/skip-") == Some(id.as_str().as_bytes()) {
                skipped.push(id);
            } else {
                return fail(BisectFailure::ForeignSession);
            }
        }
        let Some(bad) = bad else { return Ok(s) };
        if good.is_empty() {
            return Ok(s);
        }
        let range =
            self.bisect_candidates(&saved.bad, std::slice::from_ref(&saved.good), cancel)?;
        if !good.contains(&saved.good)
            || !range.contains(&bad)
            || good
                .iter()
                .chain(&skipped)
                .any(|id| *id != saved.good && !range.contains(id))
        {
            return fail(BisectFailure::ForeignSession);
        }
        s.candidates = self.bisect_candidates(&bad, &good, cancel)?;
        if s.candidates.is_empty() {
            return fail(BisectFailure::ForeignSession);
        }
        for line in meta(context, "BISECT_LOG").split(|b| *b == b'\n') {
            for (prefix, action) in [
                (b"git bisect good ".as_slice(), BisectAction::Good),
                (b"git bisect bad ".as_slice(), BisectAction::Bad),
                (b"git bisect skip ".as_slice(), BisectAction::Skip),
            ] {
                if let Some(id) = line.strip_prefix(prefix) {
                    s.decisions.push(BisectDecision {
                        action,
                        commit: parse::oid(id, "bisect decision")?,
                    });
                }
            }
        }
        if s.candidates.len() == 1 {
            s.phase = BisectPhase::Found;
            s.target = Some(bad);
        } else if s
            .candidates
            .iter()
            .all(|id| *id == bad || skipped.contains(id))
        {
            s.phase = BisectPhase::Inconclusive;
        } else {
            let candidate = meta(context, "BISECT_HEAD").trim_ascii();
            if candidate.is_empty() {
                return Ok(s);
            }
            let id = parse::oid(candidate, "bisect candidate")?;
            // A kill between recording a decision and selecting its successor
            // leaves a known good/skipped HEAD. Only recovery is offered.
            if !s.candidates.contains(&id) || skipped.contains(&id) || id == bad {
                return Ok(s);
            }
            s.target = Some(id);
            s.phase = BisectPhase::Testing;
        }
        if let Some(target) = &s.target
            && context.head
                != (Head::Detached {
                    target: target.clone(),
                })
        {
            s.phase = BisectPhase::CheckoutPending;
        }
        Ok(s)
    }
    fn bisect_candidates(
        &self,
        bad: &ObjectId,
        good: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<Vec<ObjectId>> {
        let mut input = format!("{bad}\n").into_bytes();
        for id in good {
            input.extend_from_slice(format!("^{id}\n").as_bytes());
        }
        let out = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "--no-replace-objects",
                    "rev-list",
                    "--topo-order",
                    &format!("--max-count={}", LIMIT + 1),
                    "--stdin",
                    "--",
                ]),
                input,
                cancel,
            )?
            .success("bisect candidates")?;
        let ids = out
            .split(|b| *b == b'\n')
            .filter(|b| !b.is_empty())
            .map(|b| parse::oid(b, "bisect candidates"))
            .collect::<Result<Vec<_>>>()?;
        if ids.len() > LIMIT {
            return fail(BisectFailure::RangeTooLarge);
        }
        Ok(ids)
    }
    fn bisect_supported(&self, cancel: &CancellationToken) -> Result<()> {
        if self
            .bisect_read(&["rev-parse", "--is-shallow-repository"], cancel)?
            .trim_ascii()
            != b"false"
            || !self
                .bisect_read(
                    &["for-each-ref", "--format=%(refname)", "refs/replace/"],
                    cancel,
                )?
                .is_empty()
            || self
                .info()
                .common_dir
                .join("info/grafts")
                .try_exists()
                .map_err(|e| Error::io("inspect bisect history", e))?
        {
            return fail(BisectFailure::UnsupportedHistory);
        }
        Ok(())
    }
    fn bisect_original(&self, saved: &Saved, cancel: &CancellationToken) -> Result<()> {
        if let Head::Branch { name, target } = &saved.original {
            let full = bytes_to_os(&[b"refs/heads/".as_slice(), name.as_slice()].concat())?;
            let mut arguments = args(&["show-ref", "--verify", "--hash", "--"]);
            arguments.push(full);
            let out = self
                .runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?;
            if !out.status.success() || out.stdout.trim_ascii() != target.as_str().as_bytes() {
                return fail(BisectFailure::OriginalBranchMoved);
            }
        }
        Ok(())
    }
    fn bisect_checkout(
        &self,
        head: &Head,
        label: &'static str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.bisect_context(true, cancel)?;
        let mut arguments = args(&[
            "--no-replace-objects",
            "-c",
            "submodule.recurse=false",
            "switch",
            "--no-guess",
            "--no-recurse-submodules",
            "--no-overwrite-ignore",
        ]);
        match head {
            Head::Branch { name, .. } => {
                arguments.push("--".into());
                arguments.push(self.branch_argument(name, cancel)?);
            }
            Head::Detached { target } => {
                arguments.extend(args(&["--detach", "--", target.as_str()]));
            }
            Head::Unborn { .. } => return fail(BisectFailure::NotReady),
        }
        let out = self
            .runner
            .write(self.cwd(), &arguments, Vec::new(), cancel)?;
        if !out.status.success() {
            // post-checkout can fail after HEAD/files have already changed.
            // Report that uncertain command separately from a refused move.
            if self.head(cancel).is_ok_and(|current| current == *head) {
                return fail(BisectFailure::CommandFailed);
            }
            return fail(BisectFailure::CheckoutBlocked);
        }
        if self.head(cancel)? != *head {
            return fail(BisectFailure::CommandFailed);
        }
        let _ = label;
        Ok(())
    }
    fn bisect_engine(
        &self,
        arguments: &[&str],
        label: &'static str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut argv = args(&[
            "--no-replace-objects",
            "-c",
            "submodule.recurse=false",
            "bisect",
        ]);
        argv.extend(args(arguments));
        let output = self.runner.write(self.cwd(), &argv, Vec::new(), cancel)?;
        if output.status.code() == Some(2)
            && self
                .bisect_session(cancel)?
                .is_some_and(|s| s.phase == BisectPhase::Inconclusive)
        {
            return Ok(());
        }
        output.success(label).map(|_| ())
    }
    fn bisect_revision(&self, revision: &str, cancel: &CancellationToken) -> Result<ObjectId> {
        self.resolve_commit(revision, cancel).map_err(|e| match e {
            Error::Git { .. } => Error::Bisect(BisectFailure::RevisionUnavailable),
            other => other,
        })
    }
    fn bisect_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        let mut argv = args(&["--no-replace-objects"]);
        argv.extend(args(arguments));
        self.runner
            .run(self.cwd(), &argv, Vec::new(), cancel)?
            .success("bisect state")
    }
    fn bisect_write(
        &self,
        arguments: &[&str],
        label: &'static str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.runner
            .write(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success(label)
            .map(|_| ())
    }
}
impl Saved {
    fn bytes(&self) -> Vec<u8> {
        let branch = match &self.original {
            Head::Branch { name, .. } => name.as_slice(),
            _ => b"",
        };
        [
            b"ROCKET-BISECT-1".as_slice(),
            branch,
            head_id(&self.original)
                .expect("validated original")
                .as_str()
                .as_bytes(),
            self.good.as_str().as_bytes(),
            self.bad.as_str().as_bytes(),
            self.recovery.as_bytes(),
        ]
        .join(&0)
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        let p: Vec<_> = bytes.split(|b| *b == 0).collect();
        if p.len() != 6 || p[0] != b"ROCKET-BISECT-1" {
            return fail(BisectFailure::JournalChanged);
        }
        let id = |b| {
            parse::oid(b, "bisect journal")
                .map_err(|_| Error::Bisect(BisectFailure::JournalChanged))
        };
        let target = id(p[2])?;
        let original = if p[1].is_empty() {
            Head::Detached { target }
        } else {
            Head::Branch {
                name: p[1].to_vec(),
                target,
            }
        };
        let recovery =
            std::str::from_utf8(p[5]).map_err(|_| Error::Bisect(BisectFailure::JournalChanged))?;
        if !recovery
            .strip_prefix("refs/rocket/bisect/")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        {
            return fail(BisectFailure::JournalChanged);
        }
        Ok(Self {
            original,
            good: id(p[3])?,
            bad: id(p[4])?,
            recovery: recovery.into(),
        })
    }
}
fn fail<T>(failure: BisectFailure) -> Result<T> {
    Err(Error::Bisect(failure))
}
fn head_id(head: &Head) -> Result<&ObjectId> {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => Ok(target),
        _ => fail(BisectFailure::NotReady),
    }
}
fn meta<'a>(c: &'a Context, name: &str) -> &'a [u8] {
    c.metadata
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| b.as_slice())
        .unwrap_or_default()
}
fn has(c: &Context, name: &str) -> bool {
    c.metadata.iter().any(|(n, _)| n == name)
}
