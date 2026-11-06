//! A short read of one repository for lists of many, such as the Repository manager
//! and a folder scan: its branch, how it stands against its upstream, when it last
//! fetched and, when asked, its last commit and uncommitted files. Local only: no
//! network, no optional locks, nothing written.
use crate::{runner::args, *};
use std::time::SystemTime;

/// What [`Repository::summary`] reads beyond the branch and the last fetch time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SummaryReads {
    /// The upstream, ahead/behind, uncommitted files and any operation in progress:
    /// one `git status` over the working tree, the slow part in a large checkout.
    pub status: bool,
    /// The checked-out commit's time: one more small read.
    pub last_commit: bool,
}

/// Uncommitted files: tracked ones that are changed, staged or conflicted, and new
/// ones Git doesn't track yet. Ignored files are not counted, and an untracked
/// folder counts once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SummaryChanges {
    pub tracked: usize,
    pub new: usize,
}
impl SummaryChanges {
    pub fn total(self) -> usize {
        self.tracked + self.new
    }
}

/// What [`Repository::summary`] read. Fields it was not asked for stay empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySummary {
    pub head: Head,
    /// The upstream as Git names it, for example `origin/main`.
    pub upstream: Option<Vec<u8>>,
    /// Commits only here and only on the upstream, as of the last fetch.
    pub ahead_behind: Option<(u64, u64)>,
    pub changes: Option<SummaryChanges>,
    /// A merge, rebase, cherry-pick, revert, bisect or commit import in progress.
    pub operation: OperationState,
    /// The checked-out commit's committer time, in Unix seconds.
    pub last_commit: Option<i64>,
    /// When Git last wrote FETCH_HEAD for this working tree or its repository.
    pub last_fetch: Option<SystemTime>,
}

impl Repository {
    /// Read-only summary for a list row. Without `reads.status` it never looks at the
    /// working tree. A bare repository has no working tree, so it has no changes,
    /// upstream comparison or operation, even when asked.
    pub fn summary(
        &self,
        reads: SummaryReads,
        cancel: &CancellationToken,
    ) -> Result<RepositorySummary> {
        self.runner.step("summary", "read_summary", || {
            let mut summary = RepositorySummary {
                head: Head::Unborn { branch: Vec::new() },
                upstream: None,
                ahead_behind: None,
                changes: None,
                operation: OperationState::default(),
                last_commit: None,
                last_fetch: self.last_fetch(),
            };
            if reads.status && self.info().worktree.is_some() {
                let output = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&[
                            "status",
                            "--porcelain=v2",
                            "-z",
                            "--branch",
                            "--untracked-files=normal",
                            "--ignore-submodules=none",
                        ]),
                        Vec::new(),
                        cancel,
                    )?
                    .success("summary status")?;
                let status = parse::status(&output)?;
                let mut changes = SummaryChanges::default();
                for entry in &status.entries {
                    match entry.kind {
                        EntryKind::Ignored => {}
                        EntryKind::Untracked => changes.new += 1,
                        _ => changes.tracked += 1,
                    }
                }
                summary.head = status.head;
                summary.upstream = status.upstream;
                summary.ahead_behind = status.ahead_behind;
                summary.changes = Some(changes);
                summary.operation = self.operation_state(cancel)?;
            } else {
                summary.head = self.head(cancel)?;
            }
            if reads.last_commit
                && let Head::Branch { target, .. } | Head::Detached { target } = &summary.head
            {
                summary.last_commit = Some(self.commit_time(target, cancel)?);
            }
            cancel.check()?;
            Ok(summary)
        })
    }

    /// How many commits `to` has that `from` doesn't, for example after a fast-forward.
    pub fn commits_between(
        &self,
        from: &ObjectId,
        to: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let range = format!("{}..{}", from.as_str(), to.as_str());
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-list", "--count", &range, "--"]),
                Vec::new(),
                cancel,
            )?
            .success("count commits")?;
        std::str::from_utf8(parse::line(&output))
            .ok()
            .and_then(|count| count.parse().ok())
            .ok_or_else(|| Error::parse("count commits", "not a count"))
    }

    fn commit_time(&self, commit: &ObjectId, cancel: &CancellationToken) -> Result<i64> {
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["log", "-1", "--no-show-signature", "--format=%ct", commit.as_str(), "--"]),
                Vec::new(),
                cancel,
            )?
            .success("summary commit time")?;
        std::str::from_utf8(parse::line(&output))
            .ok()
            .and_then(|time| time.parse().ok())
            .ok_or_else(|| Error::parse("summary commit time", "not a time"))
    }

    /// FETCH_HEAD belongs to the working tree that fetched; a linked worktree's
    /// repository may have fetched from another one, so the later of the two counts.
    fn last_fetch(&self) -> Option<SystemTime> {
        let info = self.info();
        [&info.git_dir, &info.common_dir]
            .into_iter()
            .filter_map(|directory| std::fs::metadata(directory.join("FETCH_HEAD")).ok()?.modified().ok())
            .max()
    }
}
