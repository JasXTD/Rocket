//! How each local branch stands against what Git's ordinary deletion checks.
use crate::{runner::args, *};

/// Unmerged branches past this many are reported without a commit count.
const COUNTED: usize = 50;

/// A local branch as `git branch --delete` judges it: against its upstream when one
/// is configured and fetched, otherwise against HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchMergeState {
    /// Short name, as in `refs/heads/<name>`.
    pub name: Vec<u8>,
    /// Git would delete it: every commit is on the checked commit.
    pub merged: bool,
    /// Commits only on this branch, when counted (0 when merged).
    pub unmerged: Option<u64>,
    /// Checked against its upstream rather than HEAD.
    pub upstream: bool,
    /// The branch's last commit, which the loaded history may not include.
    pub tip: Commit,
}

impl Repository {
    /// Read-only. Whether `git branch --delete` would accept each named local branch
    /// now, and how many commits it would refuse to lose. Names that are not local
    /// branches are left out; the answer can change before any deletion runs.
    pub fn branch_merge_states(
        &self,
        names: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<Vec<BranchMergeState>> {
        self.runner.step("branch-merge-state", "read_merge_state", || {
            if names.len() > MAX_BRANCH_BATCH {
                return Err(Error::InvalidInput(format!(
                    "check at most {MAX_BRANCH_BATCH} branches at once"
                )));
            }
            let branches = self.branches(cancel)?;
            let head = match self.head(cancel)? {
                Head::Branch { target, .. } | Head::Detached { target } => Some(target),
                Head::Unborn { .. } => None,
            };
            let target_of = |reference: &[u8]| {
                branches
                    .iter()
                    .find(|b| b.reference == reference)
                    .map(|b| b.target.clone())
            };
            let mut found = Vec::new();
            let mut counted = 0;
            for name in names {
                let mut full = b"refs/heads/".to_vec();
                full.extend_from_slice(name);
                let Some(branch) = branches
                    .iter()
                    .find(|b| b.kind == BranchKind::Local && b.reference == full)
                else {
                    continue;
                };
                let upstream = branch.upstream.as_deref().and_then(target_of);
                let against_upstream = upstream.is_some();
                let Some(base) = upstream.or_else(|| head.clone()) else {
                    // No first commit yet and no upstream: Git refuses the deletion.
                    found.push((name, &branch.target, false, None, false));
                    continue;
                };
                let (merged, unmerged) = if counted < COUNTED {
                    let count = self.count_only_on(&base, &branch.target, cancel)?;
                    if count > 0 {
                        counted += 1;
                    }
                    (count == 0, Some(count))
                } else {
                    let merged = self.is_ancestor(&branch.target, &base, cancel)?;
                    (merged, merged.then_some(0))
                };
                found.push((name, &branch.target, merged, unmerged, against_upstream));
            }
            // Every tip in one read; validated object IDs reach Git through stdin.
            let ids: Vec<ObjectId> = found.iter().map(|(_, tip, ..)| (*tip).clone()).collect();
            let tips = if ids.is_empty() {
                Vec::new()
            } else {
                let objects: Vec<u8> = ids
                    .iter()
                    .flat_map(|id| id.as_str().bytes().chain(std::iter::once(b'\n')))
                    .collect();
                let output = self
                    .runner
                    .run(self.cwd(), &args(&["cat-file", "--batch", "--buffer"]), objects, cancel)?
                    .success("branch merge state")?;
                parse::commits(&output, &ids)?
            };
            Ok(found
                .into_iter()
                .zip(tips)
                .map(|((name, _, merged, unmerged, upstream), tip)| BranchMergeState {
                    name: name.clone(),
                    merged,
                    unmerged,
                    upstream,
                    tip,
                })
                .collect())
        })
    }

    /// Commits reachable from `tip` but not from `base`.
    fn count_only_on(
        &self,
        base: &ObjectId,
        tip: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let range = format!("{base}..{tip}");
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-list", "--count", &range, "--"]),
                Vec::new(),
                cancel,
            )?
            .success("branch merge state")?;
        std::str::from_utf8(parse::line(&output))
            .ok()
            .and_then(|count| count.parse().ok())
            .ok_or_else(|| Error::parse("branch merge state", "invalid commit count"))
    }

    fn is_ancestor(
        &self,
        commit: &ObjectId,
        of: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let output = self.runner.run(
            self.cwd(),
            &args(&["merge-base", "--is-ancestor", commit.as_str(), of.as_str()]),
            Vec::new(),
            cancel,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => output.success("branch merge state").map(|_| false),
        }
    }
}
