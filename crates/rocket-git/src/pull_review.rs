//! Fetch and integration are separate, reviewed phases. Only a successful, parsed
//! fetch can create a ticket; strategy changes reuse that ticket without transport.
use crate::*;
use std::{path::PathBuf, sync::mpsc::SyncSender};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullStrategy {
    #[default]
    FastForward,
    Merge,
    Rebase,
}
impl PullStrategy {
    pub fn code(self) -> &'static str {
        match self {
            Self::FastForward => "fast-forward",
            Self::Merge => "merge",
            Self::Rebase => "rebase",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::FastForward => "Fast-forward only",
            Self::Merge => "Merge when needed",
            Self::Rebase => "Rebase local commits",
        }
    }
}
#[derive(Clone, Debug)]
pub struct PullFetched {
    repository: PathBuf,
    upstream: PullUpstream,
    remote: Remote,
    original: ObjectId,
    target: ObjectId,
}
impl PullFetched {
    pub fn upstream(&self) -> &PullUpstream {
        &self.upstream
    }
    pub fn original(&self) -> &ObjectId {
        &self.original
    }
    pub fn target(&self) -> &ObjectId {
        &self.target
    }
}
#[derive(Debug)]
#[must_use = "fetch may have updated objects/refs; inspect prepared review and every observation"]
pub struct PullFetchReport {
    pub fetch: Result<FetchOutput>,
    pub prepared: Option<Result<PullFetched>>,
    pub status: Result<Status>,
    pub branches: Result<Vec<Branch>>,
    pub state: Result<OperationState>,
}
/// A fetch-only pull result with the caller's fresh reconciliation. No merge or
/// rebase has started; transport and review failures remain separate from reads.
#[derive(Debug)]
#[must_use = "inspect transport, prepared review and reconciliation independently"]
pub struct PullFetchRefresh<T> {
    pub fetch: Result<FetchOutput>,
    pub prepared: Option<Result<PullFetched>>,
    pub refreshed: T,
}
#[derive(Clone, Debug)]
pub struct PullPreview {
    fetched: PullFetched,
    strategy: PullStrategy,
    merge: MergePreview,
    rebase: Option<RebasePreview>,
}
impl PullPreview {
    pub fn fetched(&self) -> &PullFetched {
        &self.fetched
    }
    pub fn strategy(&self) -> PullStrategy {
        self.strategy
    }
    pub fn target(&self) -> &Commit {
        self.merge.target()
    }
    pub fn incoming(&self) -> u64 {
        self.merge.incoming()
    }
    pub fn current_only(&self) -> u64 {
        self.merge.current_only()
    }
    pub fn relation(&self) -> MergeRelation {
        self.merge.relation()
    }
    pub fn can_apply(&self) -> bool {
        self.merge.can_apply()
    }
    /// Prediction for replaying local commits onto the pinned fetched object.
    pub fn prediction(&self) -> Option<&CommitPrediction> {
        self.rebase.as_ref().and_then(RebasePreview::prediction)
    }
    pub fn replay(&self) -> &[Commit] {
        self.rebase.as_ref().map_or(&[], |p| p.commits())
    }
    pub fn recovery_reference(&self) -> Option<String> {
        self.rebase.as_ref().map(|p| p.recovery_reference())
    }
}
#[derive(Debug)]
#[must_use = "inspect command, refresh and recovery state; cancellation is not rollback"]
pub enum PullApplyReport {
    Merge(Box<MergeWriteReport>),
    Rebase(Box<RebaseWriteReport>),
}
impl Repository {
    /// Fetch only. Never touches the checkout/index or starts merge/rebase. Initial
    /// dirty state, hidden index flags and active operations are refused before transport.
    pub fn fetch_for_pull(
        &self,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<PullFetchReport> {
        let report = self.fetch_for_pull_with_refresh(progress, cancel, |repo| {
            let status = repo.runner.observe("refresh", "status", || {
                repo.status(&CancellationToken::new())
            });
            let branches = repo.runner.observe("refresh", "branches", || {
                repo.branches(&CancellationToken::new())
            });
            let state = repo.runner.observe("refresh", "operation", || {
                repo.operation_state(&CancellationToken::new())
            });
            (status, branches, state)
        })?;
        let (status, branches, state) = report.refreshed;
        Ok(PullFetchReport {
            fetch: report.fetch, prepared: report.prepared, status, branches, state,
        })
    }

    /// Like `fetch_for_pull`, using the caller's fresh reconciliation while the
    /// write permit is held. The callback runs even after transport cancellation
    /// or failure; it must be read-only (no nested write permit) and use an
    /// independent cancellation token.
    pub fn fetch_for_pull_with_refresh<T>(
        &self,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
        refresh: impl FnOnce(&Self) -> T,
    ) -> Result<PullFetchRefresh<T>> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let (branch, original) =
            self.runner
                .step("pull-fetch", "validate_pull_checkout", || {
                    match self.pull_merge_context(cancel)?.head {
                        Head::Branch { name, target } => Ok((name, target)),
                        _ => Err(Error::Pull(PullFailure::StateChanged)),
                    }
                })?;
        let (upstream, remote) = self.pull_configuration(&branch, cancel)?;
        let (arguments, input) = self.prepare_pull_fetch(&upstream, cancel)?;
        if self.pull_configuration(&branch, cancel)? != (upstream.clone(), remote.clone()) {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        // After transport starts every return is a report, including cancellation.
        let fetch = self.execute_fetch(&arguments, input, progress, cancel);
        let prepared = match &fetch {
            Ok(FetchOutput {
                updates: Ok(updates),
                ..
            }) => Some(
                self.runner
                    .step("pull-fetch", "prepare_fetched_review", || {
                        let target = self.fetched_pull_target(&upstream, updates, cancel)?;
                        let fetched = PullFetched {
                            repository: self.info().git_dir.clone(),
                            upstream,
                            remote,
                            original,
                            target,
                        };
                        self.validate_pull_fetched(&fetched, cancel)?;
                        Ok(fetched)
                    }),
            ),
            _ => None,
        };
        Ok(PullFetchRefresh { fetch, prepared, refreshed: refresh(self) })
    }
    /// Local-only review of the exact fetched object. Choosing another strategy
    /// does not fetch again or trust the potentially stale current tracking ref.
    pub fn preview_pull(
        &self,
        fetched: &PullFetched,
        strategy: PullStrategy,
        cancel: &CancellationToken,
    ) -> Result<PullPreview> {
        self.runner.step("pull-preview", "review_integration", || {
            let context = self.validate_pull_fetched(fetched, cancel)?;
            let mode = if strategy == PullStrategy::FastForward {
                MergeMode::FastForwardOnly
            } else {
                MergeMode::AllowFastForward
            };
            let merge = self.describe_merge(context, fetched.target.as_str(), mode, false, cancel)?;
            let rebase = if strategy == PullStrategy::Rebase {
                Some(self.preview_rebase(RebaseAction::Start, fetched.target.as_str(), cancel)?)
            } else {
                None
            };
            let current = self.validate_pull_fetched(fetched, cancel)?;
            self.validate_merge_context(&merge, &current, cancel)?;
            Ok(PullPreview {
                fetched: fetched.clone(),
                strategy,
                merge,
                rebase,
            })
        })
    }
    /// One gate covers upstream/state validation and the existing merge/rebase
    /// transaction. No second fetch, autostash, automatic retry or rollback.
    pub fn apply_pull(
        &self,
        preview: &PullPreview,
        cancel: &CancellationToken,
    ) -> Result<PullApplyReport> {
        self.require_worktree(cancel)?;
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("pull-apply", "revalidate_pull_review", || {
                self.validate_pull_fetched(&preview.fetched, cancel)
            })?;
        self.runner.step(
            "pull-apply",
            match preview.strategy {
                PullStrategy::FastForward => "integrate_fast_forward",
                PullStrategy::Merge => "integrate_merge",
                PullStrategy::Rebase => "integrate_rebase",
            },
            || {
                if let Some(rebase) = &preview.rebase {
                    self.apply_rebase_locked(rebase, cancel)
                        .map(|r| PullApplyReport::Rebase(Box::new(r)))
                } else {
                    self.apply_merge_locked(&preview.merge, cancel)
                        .map(|r| PullApplyReport::Merge(Box::new(r)))
                }
            },
        )
    }
    fn validate_pull_fetched(
        &self,
        fetched: &PullFetched,
        cancel: &CancellationToken,
    ) -> Result<crate::merge_review::Context> {
        cancel.check()?;
        if fetched.repository != self.info().git_dir {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        let context = self.pull_merge_context(cancel)?;
        if context.head != (Head::Branch {
                name: fetched.upstream.branch.clone(), target: fetched.original.clone(),
            })
            || self.pull_configuration(&fetched.upstream.branch, cancel)?
                != (fetched.upstream.clone(), fetched.remote.clone())
        {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        self.require_direct_pull_ref(&fetched.upstream.tracking_reference, cancel)?;
        let reference = model::bytes_to_os(&fetched.upstream.tracking_reference)?;
        let output = self.runner.run(
            self.cwd(),
            &[
                "show-ref".into(),
                "--verify".into(),
                "--hash".into(),
                "--".into(),
                reference,
            ],
            Vec::new(),
            cancel,
        )?;
        if !output.status.success()
            || parse::line(&output.stdout) != fetched.target.as_str().as_bytes()
        {
            return Err(Error::Pull(PullFailure::StateChanged));
        }
        Ok(context)
    }

    /// One effective Git config read per checkpoint, including includes. No
    /// config is cached across fetch/review/apply; duplicate values still fail.
    fn pull_configuration(
        &self, branch: &[u8], cancel: &CancellationToken,
    ) -> Result<(PullUpstream, Remote)> {
        // Query only this literal branch; unrelated branch settings must not
        // consume the bounded output budget in repositories with many branches.
        let mut pattern = b"^(branch\\.".to_vec();
        for &byte in branch {
            if b".[]\\*^$()+?{}|".contains(&byte) { pattern.push(b'\\'); }
            pattern.push(byte);
        }
        pattern.extend_from_slice(b"\\.(remote|merge)|remote\\..*\\.(url|pushurl|fetch))$");
        let mut arguments = crate::runner::args(&["config", "--null", "--get-regexp"]);
        arguments.push(crate::model::bytes_to_os(&pattern)?);
        let output = self.runner.run(self.cwd(), &arguments, Vec::new(), cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Err(Error::Pull(PullFailure::MissingUpstream));
        }
        let bytes = output.success("read pull configuration")?;
        let (remote, reference, remotes) = pull_configuration_values(branch, &bytes)?;
        let upstream = self.checked_pull_upstream(branch, remote, reference, cancel)?;
        let remote = crate::remote::parse_remotes(&remotes)?.into_iter()
            .find(|r| r.name == upstream.remote)
            .ok_or(Error::Pull(PullFailure::StateChanged))?;
        Ok((upstream, remote))
    }
}

fn pull_configuration_values(branch: &[u8], bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let records = bytes.strip_suffix(b"\0")
        .ok_or_else(|| Error::parse("pull configuration", "unterminated config record"))?;
    let remote_key = [b"branch.", branch, b".remote"].concat();
    let merge_key = [b"branch.", branch, b".merge"].concat();
    let mut remote = Vec::new();
    let mut reference = Vec::new();
    let mut remotes = Vec::new();
    for record in records.split(|b| *b == 0) {
        let split = record.iter().position(|b| *b == b'\n').unwrap_or(record.len());
        let key = &record[..split];
        let value = record.get(split + 1..).unwrap_or_default();
        if key == remote_key { remote.push(value); }
        else if key == merge_key { reference.push(value); }
        else if key.starts_with(b"remote.") {
            remotes.extend_from_slice(record);
            remotes.push(0);
        }
        else if !(key.starts_with(b"branch.")
            && (key.ends_with(b".remote") || key.ends_with(b".merge")))
        {
            return Err(Error::parse("pull configuration", "unexpected config record"));
        }
    }
    fn single(values: Vec<&[u8]>) -> Result<Vec<u8>> {
        match values.as_slice() {
            [] | [b""] => Err(Error::Pull(PullFailure::MissingUpstream)),
            [value] if value.len() <= 4096 => Ok(value.to_vec()),
            _ => Err(Error::Pull(PullFailure::UnsupportedUpstream)),
        }
    }
    Ok((single(remote)?, single(reference)?, remotes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_pull_config_keeps_literal_names_and_remote_values() {
        let input = "branch.Topic.文.remote\nOrigin.Team\0branch.Topic.文.merge\nrefs/heads/main\0branch.topic.文.remote\nother\0remote.Origin.Team.url\nfirst\nsecond\0remote.Origin.Team.fetch\n+refs/heads/*:refs/remotes/Origin.Team/*\0";
        let (name, reference, remotes) = pull_configuration_values("Topic.文".as_bytes(), input.as_bytes()).unwrap();
        assert_eq!(name, b"Origin.Team");
        assert_eq!(reference, b"refs/heads/main");
        assert_eq!(crate::remote::parse_remotes(&remotes).unwrap()[0].urls, [b"first\nsecond".to_vec()]);
    }

    #[test]
    fn effective_pull_config_refuses_ambiguous_empty_and_malformed_records() {
        let valid = b"branch.main.remote\norigin\0branch.main.merge\nrefs/heads/main\0";
        for extra in [b"branch.main.remote\norigin\0".as_slice(), b"branch.main.merge\nrefs/heads/other\0"] {
            let result = pull_configuration_values(b"main", &[valid.as_slice(), extra].concat());
            assert!(matches!(result, Err(Error::Pull(PullFailure::UnsupportedUpstream))));
        }
        assert!(matches!(pull_configuration_values(b"main", b"branch.main.remote\0branch.main.merge\nrefs/heads/main\0"), Err(Error::Pull(PullFailure::MissingUpstream))));
        assert!(pull_configuration_values(b"main", &valid[..valid.len()-1]).is_err());
        assert!(pull_configuration_values(b"main", &[valid.as_slice(), b"unexpected\0"].concat()).is_err());
        let oversized = format!("branch.main.remote\n{}\0branch.main.merge\nrefs/heads/main\0", "a".repeat(4097));
        assert!(matches!(pull_configuration_values(b"main", oversized.as_bytes()), Err(Error::Pull(PullFailure::UnsupportedUpstream))));
    }
}
