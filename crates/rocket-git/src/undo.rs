//! Session-owned undo of a normal Rocket commit. No worktree or index writes.
use crate::{commit_actions::WorkingFile, local_files, runner::args, *};
use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Created only after a confirmed ordinary commit through `commit_with_undo`.
#[derive(Clone, Debug)]
pub struct UndoCommit {
    repository: PathBuf,
    branch: Vec<u8>,
    before: Option<ObjectId>,
    after: ObjectId,
    index: Vec<u8>,
    subject: Vec<u8>,
}
impl UndoCommit {
    pub fn commit(&self) -> &ObjectId {
        &self.after
    }
    pub fn previous(&self) -> Option<&ObjectId> {
        self.before.as_ref()
    }
    pub fn branch(&self) -> &[u8] {
        &self.branch
    }
    pub fn subject(&self) -> &[u8] {
        &self.subject
    }
    /// SHA-256 of the index right after the commit. With the branch and commit it
    /// pins the record across processes; see [`Repository::pinned_undo_commit`].
    pub fn index_digest(&self) -> Result<String> {
        crate::sha256_bytes(&self.index)
    }
}
pub struct UndoableCommitReport {
    pub write: WriteReport,
    pub undo: Option<UndoCommit>,
    /// Failure to prepare optional Undo never changes a successful commit result.
    pub undo_unavailable: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Guard {
    head: Head,
    index: Vec<u8>,
    status: Vec<u8>,
    files: Vec<(RepoPath, WorkingFile)>,
}
#[derive(Clone, Debug)]
pub struct UndoCommitPreview {
    record: UndoCommit,
    guard: Guard,
    reviewed: Instant,
    recovery: String,
}
impl UndoCommitPreview {
    pub fn record(&self) -> &UndoCommit {
        &self.record
    }
    pub fn recovery_reference(&self) -> &str {
        &self.recovery
    }
}
#[derive(Debug)]
pub struct UndoCommitReport {
    pub result: Result<()>,
    pub verified: Result<()>,
    pub status: Result<Status>,
    pub recovery_reference: String,
}
fn invalid(code: &str, message: &str) -> Error {
    Error::InvalidInput(format!("[{code}] {message}"))
}
/// Only fixed codes enter diagnostic logs; never commit messages or paths.
pub fn failure_code(message: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
        "undo_branch_required",
        "undo_unavailable",
        "undo_changed",
        "undo_not_ordinary",
        "undo_wrong_repository",
        "undo_active_operation",
        "undo_head_changed",
        "undo_index_changed",
        "undo_shared_commit",
        "undo_stale_preview",
        "undo_clock",
        "undo_review_expired",
        "undo_locked",
        "undo_not_confirmed",
    ];
    let code = message.strip_prefix('[')?.split_once("] ")?.0;
    CODES.iter().copied().find(|known| *known == code)
}
impl Repository {
    /// Ordinary commit with a best-effort, session-only Undo capability. Existing
    /// hooks, signing and secret protection are retained by the normal writer.
    pub fn commit_with_undo(
        &self,
        message: &str,
        cancel: &CancellationToken,
    ) -> Result<UndoableCommitReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let before = self.head(cancel)?;
        let idle = self.operation_state(cancel)?.is_idle();
        let write = self.commit_locked(message, cancel)?;
        let mut report = UndoableCommitReport {
            write,
            undo: None,
            undo_unavailable: None,
        };
        if report.write.result.is_ok()
            && idle
            && let Ok(status) = &report.write.status
        {
            let captured = self.runner.step("commit", "capture_undo", || {
                let (branch, previous) = match &before {
                    Head::Branch { name, target } => (name, Some(target.clone())),
                    Head::Unborn { branch } => (branch, None),
                    Head::Detached { .. } => return Err(invalid("undo_branch_required", "Undo requires a local branch.")),
                };
                let Head::Branch { name, target } = &status.head else { return Err(invalid("undo_unavailable", "Commit checkout could not be confirmed.")); };
                if name != branch || self.head(cancel)? != status.head { return Err(invalid("undo_changed", "Checkout changed while recording Undo.")); }
                let commit = self.history(HistoryStart::Commit(target.clone()), 1, cancel)?.commits.into_iter().next()
                    .ok_or_else(|| invalid("undo_unavailable", "Commit details are unavailable."))?;
                if commit.id != *target || commit.parents != previous.iter().cloned().collect::<Vec<_>>() {
                    return Err(invalid("undo_not_ordinary", "Undo supports an ordinary commit with its original parent, including a first commit."));
                }
                Ok(UndoCommit { repository: self.info().git_dir.clone(), branch: branch.clone(), before: previous,
                    after: target.clone(), index: self.undo_read(&["ls-files", "--stage", "-v", "-z"], cancel)?, subject: commit.message.split(|b| *b == b'\n').next().unwrap_or_default().to_vec() })
            });
            match captured {
                Ok(record) => report.undo = Some(record),
                Err(_) => report.undo_unavailable = Some("Commit completed; optional Undo could not be prepared. Use Reflog & Recovery if needed.".into()),
            }
        }
        Ok(report)
    }
    /// Rebuild the Undo record of the commit another process made, from the values
    /// its `UndoCommit` reported. The branch must still be at that ordinary commit and
    /// the index must still hash to `index_digest`, so later staging is refused just
    /// as in the session that made the commit. Nothing is written.
    pub fn pinned_undo_commit(
        &self,
        branch: &[u8],
        commit: &ObjectId,
        index_digest: &str,
        cancel: &CancellationToken,
    ) -> Result<UndoCommit> {
        self.require_worktree(cancel)?;
        let expected = Head::Branch { name: branch.to_vec(), target: commit.clone() };
        if self.head(cancel)? != expected {
            return Err(invalid("undo_head_changed", "The branch or commit changed. No Undo was applied; use Reflog & Recovery."));
        }
        let details = self.history(HistoryStart::Commit(commit.clone()), 1, cancel)?.commits.into_iter().next()
            .filter(|c| c.id == *commit && c.parents.len() <= 1)
            .ok_or_else(|| invalid("undo_not_ordinary", "Undo supports an ordinary commit with its original parent, including a first commit."))?;
        let index = self.undo_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if crate::sha256_bytes(&index)? != index_digest.to_ascii_lowercase() {
            return Err(invalid("undo_index_changed", "Staging changed since this commit. Keep those changes and use Reflog & Recovery."));
        }
        Ok(UndoCommit {
            repository: self.info().git_dir.clone(),
            branch: branch.to_vec(),
            before: details.parents.first().cloned(),
            after: commit.clone(),
            index,
            subject: details.message.split(|b| *b == b'\n').next().unwrap_or_default().to_vec(),
        })
    }
    fn undo_read(&self, arguments: &[&str], cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), Vec::new(), cancel)?
            .success("undo-read")
    }
    fn undo_guard(&self, record: &UndoCommit, cancel: &CancellationToken) -> Result<Guard> {
        self.require_worktree(cancel)?;
        if self.info().git_dir != record.repository {
            return Err(invalid(
                "undo_wrong_repository",
                "Undo belongs to another checkout.",
            ));
        }
        if !self.operation_state(cancel)?.is_idle() {
            return Err(invalid(
                "undo_active_operation",
                "Finish or abort the active Git operation before Undo.",
            ));
        }
        let status = self.status(cancel)?;
        if status.head
            != (Head::Branch {
                name: record.branch.clone(),
                target: record.after.clone(),
            })
        {
            return Err(invalid(
                "undo_head_changed",
                "The branch or commit changed. No Undo was applied; use Reflog & Recovery.",
            ));
        }
        let index = self.undo_read(&["ls-files", "--stage", "-v", "-z"], cancel)?;
        if index != record.index {
            return Err(invalid(
                "undo_index_changed",
                "Staging changed since this commit. Keep those changes and use Reflog & Recovery.",
            ));
        }
        if !self
            .undo_read(
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    &format!("--contains={}", record.after),
                    "refs/remotes/",
                ],
                cancel,
            )?
            .is_empty()
        {
            return Err(invalid(
                "undo_shared_commit",
                "A known remote-tracking branch contains this commit. Use Revert for a shared commit.",
            ));
        }
        let files = self.snapshot_changed_files(&status, cancel)?;
        let bytes = self.undo_read(
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=no",
                "--ignore-submodules=none",
            ],
            cancel,
        )?;
        Ok(Guard {
            head: status.head,
            index,
            status: bytes,
            files,
        })
    }
    pub fn preview_undo_commit(
        &self,
        record: &UndoCommit,
        cancel: &CancellationToken,
    ) -> Result<UndoCommitPreview> {
        self.runner.step("undo-preview", "review_last_commit", || {
            let guard = self.undo_guard(record, cancel)?;
            if self.undo_guard(record, cancel)? != guard {
                return Err(invalid(
                    "undo_stale_preview",
                    "Local changes moved during review. Review again.",
                ));
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid("undo_clock", "System clock is unavailable."))?
                .as_nanos();
            Ok(UndoCommitPreview {
                record: record.clone(),
                guard,
                reviewed: Instant::now(),
                recovery: format!("refs/rocket/undo/{}-{stamp}", std::process::id()),
            })
        })
    }
    pub fn apply_undo_commit(
        &self,
        preview: &UndoCommitPreview,
        cancel: &CancellationToken,
    ) -> Result<UndoCommitReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        if preview.reviewed.elapsed() > Duration::from_secs(120) {
            return Err(invalid(
                "undo_review_expired",
                "This review expired. Review Undo again.",
            ));
        }
        // The index lock blocks normal checkout/index writers. On Windows also
        // pin HEAD against replacement. The ref transaction compares the exact
        // reviewed branch tip; it never follows a newly selected HEAD.
        let _pins = local_files::pin_directories(&self.info().git_dir, &self.info().git_dir)?;
        if self
            .info()
            .git_dir
            .join("HEAD.lock")
            .try_exists()
            .map_err(|e| Error::io("inspect Undo HEAD lock", e))?
        {
            return Err(invalid(
                "undo_locked",
                "Another Git operation holds HEAD.lock. Existing locks are preserved.",
            ));
        }
        let locks = OwnedLock::acquire(self.info().git_dir.join("index.lock"))?;
        let _head_pin = pin_head(&self.info().git_dir.join("HEAD"))?;
        self.runner.step("undo-commit", "revalidate_undo", || {
            if self.undo_guard(&preview.record, cancel)? != preview.guard {
                return Err(invalid(
                    "undo_stale_preview",
                    "Local changes changed after review. Nothing was undone; review again.",
                ));
            }
            Ok(())
        })?;
        cancel.check()?;
        let record = &preview.record;
        let mut input = b"start\n".to_vec();
        input.extend_from_slice(if record.before.is_some() {
            b"update refs/heads/"
        } else {
            b"delete refs/heads/"
        });
        input.extend_from_slice(&record.branch);
        if let Some(before) = &record.before {
            input.extend_from_slice(format!(" {before}").as_bytes());
        }
        input.extend_from_slice(format!(" {}\nprepare\ncommit\n", record.after).as_bytes());
        // Persist recovery before moving the branch. A process killed during a
        // multi-ref transaction can leave some ref files renamed and others not.
        // Keeping these steps separate guarantees the old commit already has a
        // durable name when the branch moves. Never replace a pre-existing ref.
        let result = (|| {
            self.runner.step("undo-commit", "save_undo_recovery", || {
                self.runner
                    .write(
                        self.cwd(),
                        &args(&[
                            "update-ref",
                            "--no-deref",
                            &preview.recovery,
                            record.after.as_str(),
                            &"0".repeat(record.after.as_str().len()),
                        ]),
                        Vec::new(),
                        cancel,
                    )?
                    .success("undo-recovery")?;
                if self.resolve_commit(&preview.recovery, cancel)? != record.after {
                    return Err(invalid(
                        "undo_not_confirmed",
                        "Recovery could not be confirmed. The branch was not moved.",
                    ));
                }
                Ok(())
            })?;
            self.runner.step("undo-commit", "revalidate_before_undo", || {
                if self.undo_guard(record, cancel)? != preview.guard {
                    return Err(invalid("undo_stale_preview", "Local changes moved while saving recovery. The branch was not moved; review again."));
                }
                Ok(())
            })?;
            self.runner
                .step("undo-commit", "update_branch_keep_changes", || {
                    self.runner
                        .write(
                            self.cwd(),
                            &args(&[
                                "update-ref",
                                "--no-deref",
                                "-m",
                                "Rocket Undo commit; changes kept",
                                "--stdin",
                            ]),
                            input,
                            cancel,
                        )?
                        .success("undo-commit")
                        .map(|_| ())
                })
        })();
        drop(locks);
        let fresh = CancellationToken::new();
        let status = self
            .runner
            .step("undo-commit", "refresh_status", || self.status(&fresh));
        let verified = self.runner.step("undo-commit", "verify_undo_result", || {
            let expected = match &record.before { Some(target) => Head::Branch { name: record.branch.clone(), target: target.clone() }, None => Head::Unborn { branch: record.branch.clone() } };
            if self.head(&fresh)? != expected || self.undo_read(&["ls-files", "--stage", "-v", "-z"], &fresh)? != record.index
                || self.resolve_commit(&preview.recovery, &fresh)? != record.after {
                return Err(invalid("undo_not_confirmed", "Undo result is not confirmed. Inspect the branch and recovery reference before retrying."));
            }
            Ok(())
        });
        Ok(UndoCommitReport {
            result,
            verified,
            status,
            recovery_reference: preview.recovery.clone(),
        })
    }
}
fn pin_head(path: &std::path::Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(|e| Error::io("inspect Undo HEAD", e))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(invalid(
            "undo_head_changed",
            "HEAD is not a regular file. Undo was not applied.",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid(
                "undo_head_changed",
                "HEAD is a reparse point. Undo was not applied.",
            ));
        }
        // FILE_SHARE_READ: allow Git's reads and reflog update, but prevent a
        // concurrent direct symbolic-ref writer from replacing this checkout.
        options.share_mode(1);
    }
    options
        .open(path)
        .map_err(|e| Error::io("pin Undo HEAD", e))
}
struct OwnedLock {
    path: PathBuf,
    file: Option<File>,
}
impl OwnedLock {
    fn acquire(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists { invalid("undo_locked", "Another Git operation holds a lock. Wait for it; existing locks are preserved.") }
            else { Error::io("acquire Undo lock", e) }
        })?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}
impl Drop for OwnedLock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
