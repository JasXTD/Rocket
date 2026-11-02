//! GitHub synchronous merge with a reviewed head condition and no automatic retry.
use super::*;
const PREVIEW: &str = "hosting-merge-preview";
const SUBMIT: &str = "hosting-merge-submit";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeMergeMethod {
    #[default]
    Merge,
    Squash,
    Rebase,
}
impl ForgeMergeMethod {
    pub fn key(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge commit",
            Self::Squash => "Squash and merge",
            Self::Rebase => "Rebase and merge",
        }
    }
    fn allowed_field(self) -> &'static str {
        match self {
            Self::Merge => "allow_merge_commit",
            Self::Squash => "allow_squash_merge",
            Self::Rebase => "allow_rebase_merge",
        }
    }
}
pub struct ForgePullMergePreview {
    repository: ForgeRepository,
    summary: ForgePullSummary,
    method: ForgeMergeMethod,
    session: u64,
    created: Instant,
    used: AtomicBool,
}
impl ForgePullMergePreview {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn summary(&self) -> &ForgePullSummary {
        &self.summary
    }
    pub fn method(&self) -> ForgeMergeMethod {
        self.method
    }
}
#[derive(Debug)]
pub struct ForgePullMergeResult {
    pub commit: String,
}
impl ForgeSession {
    fn merge_allowed(
        &self,
        op: &'static str,
        repository: &ForgeRepository,
        summary: &ForgePullSummary,
        method: ForgeMergeMethod,
        c: &CancellationToken,
    ) -> Result<()> {
        if !self.config.provider().is_github(){return self.provider_merge_allowed(op,repository,summary,method,c);}
        // Unknown/calculating, conflicting, blocked and unstable statuses are
        // not silently treated as eligible. GitHub still decides at submission.
        if summary.state != "open"
            || summary.draft
            || summary.merged
            || summary.mergeable != Some(true)
            || summary.mergeable_state.as_deref() != Some("clean")
        {
            return fail(ForgeFailure::PullMergeBlocked);
        }
        let j = self
            .request(
                op,
                "read_merge_permissions",
                "GET",
                &format!("/repos/{}", repository.encoded()),
                None,
                false,
                c,
            )?
            .0;
        if !j
            .str("full_name")
            .is_some_and(|n| n.eq_ignore_ascii_case(repository.path()))
        {
            return fail(ForgeFailure::InvalidResponse);
        }
        if j.bool("archived") != Some(false)
            || j.bool(method.allowed_field()) != Some(true)
            || j.get("permissions").and_then(|p| p.bool("push")) != Some(true)
        {
            return fail(ForgeFailure::PullMergeBlocked);
        }
        Ok(())
    }
    pub fn preview_pull_merge(
        &self,
        details: &ForgePullDetails,
        method: ForgeMergeMethod,
        c: &CancellationToken,
    ) -> Result<ForgePullMergePreview> {
        self.runner.step(PREVIEW, "validate_pull_merge", || {
            if details.session != self.id {
                return fail(ForgeFailure::PullReviewStale);
            }
            let current = self.current_pull(PREVIEW, &details.repository, details.summary.number, c)?;
            if !same_revision(&current, &details.summary) {
                return fail(ForgeFailure::PullChanged);
            }
            self.merge_allowed(PREVIEW, &details.repository, &current, method, c)?;
            Ok(ForgePullMergePreview {
                repository: details.repository.clone(),
                summary: current,
                method,
                session: self.id,
                created: Instant::now(),
                used: AtomicBool::new(false),
            })
        })
    }
    pub fn merge_pull(
        &self,
        preview: &ForgePullMergePreview,
        c: &CancellationToken,
    ) -> Result<ForgePullMergeResult> {
        self.runner.step(SUBMIT, "submit_pull_merge", || {
            if preview.session != self.id
                || preview.created.elapsed() > Duration::from_secs(120)
                || preview.used.load(Ordering::Acquire)
            {
                return fail(ForgeFailure::PullReviewStale);
            }
            let current = self.current_pull(SUBMIT, &preview.repository, preview.summary.number, c)?;
            if !same_revision(&current, &preview.summary) {
                return fail(ForgeFailure::PullChanged);
            }
            self.merge_allowed(SUBMIT, &preview.repository, &current, preview.method, c)?;
            c.check()?;
            if preview.used.swap(true, Ordering::AcqRel) {
                return fail(ForgeFailure::PullReviewStale);
            }
            if !self.config.provider().is_github(){return self.provider_merge(preview.repository(),&current,preview.method,c);}
            let path = self.pull_scope(&preview.repository, preview.summary.number)?;
            // sha is GitHub's server-side head compare. There is no equivalent
            // base-SHA condition in this endpoint; that remaining race is shown
            // in the preview. No branch-delete, queue or bypass action is sent.
            let body = format!(
                "{{\"sha\":{},\"merge_method\":{}}}",
                quote(&preview.summary.head),
                quote(preview.method.key())
            );
            let value = self
                .request(
                    SUBMIT,
                    "merge_reviewed_head",
                    "PUT",
                    &format!("{path}/merge"),
                    Some(&body),
                    true,
                    c,
                )
                .map_err(|e| match e {
                    Error::Forge(ForgeFailure::OutcomeUnknown) => {
                        Error::Forge(ForgeFailure::PullMergeUnknown)
                    }
                    Error::Forge(ForgeFailure::Rejected) => {
                        Error::Forge(ForgeFailure::PullMergeRejected)
                    }
                    other => other,
                })?
                .0;
            if value.bool("merged") == Some(false) {
                return fail(ForgeFailure::PullMergeRejected);
            }
            if value.bool("merged") != Some(true) {
                return fail(ForgeFailure::PullMergeUnknown);
            }
            let commit = value
                .str("sha")
                .and_then(oid)
                .ok_or(Error::Forge(ForgeFailure::PullMergeUnknown))?;
            Ok(ForgePullMergeResult { commit })
        })
    }
}
