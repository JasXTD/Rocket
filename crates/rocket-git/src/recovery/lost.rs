//! Read-only object discovery. Scans never expire reflogs, run GC, or write lost-found.
use super::*;
use std::sync::mpsc::SyncSender;

pub const MAX_LOST_COMMITS: usize = 10_000;

#[derive(Clone, Debug)]
pub enum LostCommitProgress {
    Inspecting,
    Scanning,
    ReadingCommits(usize),
    Complete(usize),
}
impl LostCommitProgress {
    pub fn label(&self) -> String {
        match self {
            Self::Inspecting => "Checking repository history…".into(),
            Self::Scanning => "Scanning objects not reached by refs or reflogs…".into(),
            Self::ReadingCommits(n) => format!("Reading {n} commit records…"),
            Self::Complete(n) => format!("Scan complete · {n} lost commits"),
        }
    }
}

/// A completed, bounded scan. Pagination never silently repeats the expensive scan.
#[derive(Clone, Debug)]
pub struct LostCommitScan {
    repository: PathBuf,
    common: PathBuf,
    commits: Vec<Commit>,
}
impl LostCommitScan {
    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }
    pub fn page(&self, limit: usize) -> Result<ReflogPage> {
        if !(1..=2000).contains(&limit) {
            return Err(Error::InvalidInput(
                "lost-commit page limit must be 1..2000".into(),
            ));
        }
        Ok(ReflogPage {
            entries: self
                .commits
                .iter()
                .take(limit)
                .map(|c| commit_entry(c, Vec::new(), None))
                .collect(),
            more: self.commits.len() > limit,
        })
    }
}

impl Repository {
    pub fn scan_lost_commits(
        &self,
        cancel: &CancellationToken,
        progress: Option<&SyncSender<LostCommitProgress>>,
    ) -> Result<LostCommitScan> {
        let notify = |stage| {
            if let Some(sender) = progress {
                let _ = sender.try_send(stage);
            }
        };
        self.require_worktree(cancel)?;
        notify(LostCommitProgress::Inspecting);
        self.runner.step("lost-commits", "check_lost_history", || {
            self.lost_supported(cancel)
        })?;
        notify(LostCommitProgress::Scanning);
        let bytes = self.runner.step("lost-commits", "scan_lost_objects", || {
            self.lost_read(
                &[
                    "fsck",
                    "--full",
                    "--connectivity-only",
                    "--unreachable",
                    "--no-dangling",
                    "--no-progress",
                ],
                Vec::new(),
                cancel,
            )
            .map_err(|e| match e {
                Error::Git { .. } => Error::Recovery(RecoveryFailure::LostScanFailed),
                other => other,
            })
        })?;
        let ids = parse_lost_ids(&bytes, MAX_LOST_COMMITS, cancel)?;
        notify(LostCommitProgress::ReadingCommits(ids.len()));
        let mut commits = self.runner.step("lost-commits", "read_lost_commits", || {
            self.lost_objects(&ids, cancel)
        })?;
        commits.sort_by(|a, b| {
            b.committer
                .timestamp
                .cmp(&a.committer.timestamp)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        cancel.check()?;
        notify(LostCommitProgress::Complete(commits.len()));
        Ok(LostCommitScan {
            repository: self.info().git_dir.clone(),
            common: self.info().common_dir.clone(),
            commits,
        })
    }

    /// Review the selected commit's own change (first parent for merges), never a
    /// checkout/reset. The resulting preview pins both its content and local state.
    pub fn preview_lost_recovery(
        &self,
        scan: &LostCommitScan,
        target: &ObjectId,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<RecoveryPreview> {
        self.runner.step("lost-recovery-preview", "review_lost_commit", || {
            self.require_worktree(cancel)?;
            if scan.repository != self.info().git_dir || scan.common != self.info().common_dir {
                return refused(RecoveryFailure::StalePreview);
            }
            let commit = scan.commits.iter().find(|c| c.id == *target)
                .ok_or(Error::Recovery(RecoveryFailure::LostCommitUnavailable))?.clone();
            self.lost_target_matches(&commit, cancel)?;
            let action = RecoveryAction::Branch(name.into());
            let context = self.recovery_context(&action, cancel)?;
            self.check_recovery_branch(name, cancel)?;
            // Creating HEAD's unborn branch would change the checkout's meaning.
            if matches!(&context.head, Head::Unborn { branch: current } if current == name.as_bytes()) {
                return refused(RecoveryFailure::BranchExists);
            }
            let patch = self.lost_patch(target, cancel)?;
            if self.recovery_context(&action, cancel)? != context {
                return refused(RecoveryFailure::StalePreview);
            }
            Ok(RecoveryPreview {
                repository: self.info().git_dir.clone(), action, context,
                target: commit, patch, affected: Vec::new(), lost: true, relation: None,
            })
        })
    }

    pub(super) fn lost_patch(
        &self,
        target: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let options = DiffOptions {
            target: DiffTarget::Commit { id: target.clone() },
            ..Default::default()
        };
        let mut arguments = self.diff_arguments(&options, cancel)?;
        arguments.splice(0..0, args(&["-c", "core.quotepath=false"]));
        arguments.insert(0, "--no-replace-objects".into());
        self.runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?
            .success("lost commit preview")
            .map_err(|e| match e {
                Error::Git { .. } => Error::Recovery(RecoveryFailure::LostCommitUnavailable),
                other => other,
            })
    }

    pub(super) fn lost_target_matches(
        &self,
        target: &Commit,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.lost_supported(cancel)?;
        let commits = self.lost_objects(std::slice::from_ref(&target.id), cancel)?;
        if commits.first() != Some(target) {
            return refused(RecoveryFailure::LostCommitUnavailable);
        }
        Ok(())
    }

    fn lost_objects(&self, ids: &[ObjectId], cancel: &CancellationToken) -> Result<Vec<Commit>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let input = ids
            .iter()
            .flat_map(|id| format!("{id}\n").into_bytes())
            .collect();
        let output = self.lost_read(&["cat-file", "--batch"], input, cancel)?;
        parse::commits(&output, ids)
            .map_err(|_| Error::Recovery(RecoveryFailure::LostCommitUnavailable))
    }

    fn lost_read(
        &self,
        arguments: &[&str],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut command = args(&["--no-replace-objects"]);
        command.extend(args(arguments));
        self.runner
            .run(self.cwd(), &command, input, cancel)?
            .success("lost commit read")
    }

    fn lost_supported(&self, cancel: &CancellationToken) -> Result<()> {
        if parse::line(&self.lost_read(
            &["rev-parse", "--is-shallow-repository"],
            Vec::new(),
            cancel,
        )?) != b"false"
            || !self
                .lost_read(
                    &["for-each-ref", "--format=%(refname)", "refs/replace/"],
                    Vec::new(),
                    cancel,
                )?
                .is_empty()
        {
            return refused(RecoveryFailure::LostHistoryUnsupported);
        }
        let grafts = self.info().common_dir.join("info").join("grafts");
        match fs::symlink_metadata(&grafts) {
            Ok(_) => refused(RecoveryFailure::LostHistoryUnsupported),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io("inspect history grafts", e)),
        }
    }
}

fn parse_lost_ids(bytes: &[u8], limit: usize, cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        cancel.check()?;
        let fields = line
            .strip_suffix(b"\r")
            .unwrap_or(line)
            .split(|b| *b == b' ')
            .collect::<Vec<_>>();
        if fields.len() != 3
            || fields[0] != b"unreachable"
            || ![b"commit".as_slice(), b"tree", b"blob", b"tag"].contains(&fields[1])
        {
            return refused(RecoveryFailure::LostScanFailed);
        }
        let id = parse::oid(fields[2], "lost commits")
            .map_err(|_| Error::Recovery(RecoveryFailure::LostScanFailed))?;
        if fields[1] == b"commit" && seen.insert(id.clone()) {
            ids.push(id);
            if ids.len() > limit {
                return refused(RecoveryFailure::LostScanLimit);
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_and_oversized_scans_never_look_complete_or_empty() {
        let c = CancellationToken::new();
        assert!(matches!(
            parse_lost_ids(b"unexpected diagnostic\n", 10, &c),
            Err(Error::Recovery(RecoveryFailure::LostScanFailed))
        ));
        let bytes = format!(
            "unreachable commit {}\nunreachable commit {}\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        assert!(matches!(
            parse_lost_ids(bytes.as_bytes(), 1, &c),
            Err(Error::Recovery(RecoveryFailure::LostScanLimit))
        ));
        let bytes = format!(
            "unreachable commit {0}\nunreachable commit {0}\nunreachable blob {0}\n",
            "a".repeat(64)
        );
        assert_eq!(parse_lost_ids(bytes.as_bytes(), 1, &c).unwrap().len(), 1);
        c.cancel();
        assert!(matches!(
            parse_lost_ids(bytes.as_bytes(), 1, &c),
            Err(Error::Cancelled)
        ));
    }
}
