//! Bounded, local-only gitlink history. Missing checkouts/objects remain an
//! explicit summary alongside the original short diff; they never trigger fetch.
use crate::{parse, runner::args, *};

#[derive(Clone, Debug)]
pub struct SubmoduleDiffCommit {
    pub id: ObjectId,
    pub subject: Vec<u8>,
    pub added: bool,
}
#[derive(Clone, Debug)]
pub struct SubmoduleDiff {
    pub path: RepoPath,
    pub before: Option<ObjectId>,
    pub after: Option<ObjectId>,
    pub commits: Vec<SubmoduleDiffCommit>,
    pub notice: Option<String>,
    pub limited: bool,
}
impl Repository {
    pub fn submodule_diffs(
        &self,
        options: &DiffOptions,
        cancel: &CancellationToken,
    ) -> Result<Vec<SubmoduleDiff>> {
        self.submodule_diffs_for_files(&self.file_comparisons(options, cancel)?, cancel)
    }

    /// Reuse the raw identities already read for this comparison. Identities are
    /// repository-bound; callers still obtain fresh metadata for every load.
    pub fn submodule_diffs_for_files(
        &self,
        files: &[ImageComparison],
        cancel: &CancellationToken,
    ) -> Result<Vec<SubmoduleDiff>> {
        cancel.check()?;
        self.runner.step("submodule-diff", "read-submodule-history", || {
            let mut out = Vec::new();
            for file in files {
                file.validate(self, cancel)?;
                let Some((before, after)) = file.gitlinks() else { continue; };
                cancel.check()?;
                let mut diff = SubmoduleDiff { path: file.path().clone(), before, after, commits: Vec::new(), notice: None, limited: false };
                if out.len() >= 16 {
                    diff.notice = Some("Open this path in Compare or Changes to load its history; at most 16 submodules are expanded per read.".into());
                    diff.limited = true;
                } else {
                    match self.runner.step("submodule-diff", "read-local-commits", || self.read_submodule_history(&mut diff, cancel)) {
                        Ok(()) => (),
                        Err(Error::Cancelled) => return Err(Error::Cancelled),
                        Err(e @ (Error::TimedOut | Error::OutputLimit { .. })) => return Err(e),
                        Err(_) => diff.notice = Some("Local submodule history is unavailable. The checkout or commits may be missing, shallow, or redirected. Original gitlink IDs remain below; initialize or fetch separately, then reload.".into()),
                    }
                }
                file.validate(self, cancel)?;
                out.push(diff);
            }
            Ok(out)
        })
    }
    fn read_submodule_history(
        &self,
        diff: &mut SubmoduleDiff,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let child = self
            .submodule_read_repository(&diff.path, cancel)?
            .ok_or(Error::WorktreeRequired)?;
        // Worktree gitlinks normally contain HEAD; zero means resolve it here.
        if diff
            .after
            .as_ref()
            .is_some_and(|id| id.as_str().bytes().all(|b| b == b'0'))
        {
            diff.after = Some(child.resolve_commit("HEAD", cancel)?);
        }
        for id in [&diff.before, &diff.after].into_iter().flatten() {
            child.resolve_commit(id.as_str(), cancel)?;
        }
        let mut a = args(&[
            "--no-replace-objects",
            "log",
            "--no-show-signature",
            "--no-notes",
            "--no-decorate",
            "--format=%m%H%x00%s%x00",
            "--max-count=201",
            "--left-right",
            "--topo-order",
        ]);
        match (&diff.before, &diff.after) {
            (Some(before), Some(after)) => a.push(format!("{before}...{after}").into()),
            (None, Some(id)) | (Some(id), None) => a.push(id.as_str().into()),
            (None, None) => return Ok(()),
        }
        a.push("--".into());
        let mut bounded = child.clone();
        bounded.runner.limits.stdout_bytes = 1024 * 1024;
        let raw = bounded.read_diff("submodule history", &a, cancel)?;
        let mut fields = raw.split(|b| *b == 0);
        while let Some(record) = fields.next() {
            cancel.check()?;
            let record = record.strip_prefix(b"\n").unwrap_or(record);
            if record.is_empty() {
                break;
            }
            let subject = fields
                .next()
                .ok_or_else(|| Error::parse("submodule history", "missing subject"))?;
            if diff.commits.len() == 200 {
                diff.limited = true;
                break;
            }
            let (marker, id) = record.split_at(1);
            if !matches!(marker, b"<" | b">" | b"-") {
                return Err(Error::parse("submodule history", "invalid side"));
            }
            diff.commits.push(SubmoduleDiffCommit {
                id: parse::oid(id, "submodule history")?,
                subject: subject.to_vec(),
                added: diff.before.is_none() || (diff.after.is_some() && marker != b"<"),
            });
        }
        if diff.limited {
            diff.notice = Some(
                "Showing the first 200 commits. Open the submodule for its full history.".into(),
            );
        }
        Ok(())
    }
}
