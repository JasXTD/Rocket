//! Reviewed local fast-forward batches. No checkout, fetch or forced ref update.
use crate::{runner::args, *};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FastForwardBranch {
    pub branch: Vec<u8>,
    pub before: ObjectId,
    pub upstream: Option<Vec<u8>>,
    pub after: Option<ObjectId>,
    /// None means this row can be selected. Other rows remain visible for explanation.
    pub blocked: Option<&'static str>,
}
#[derive(Clone)]
pub struct FastForwardPreview {
    repository: PathBuf,
    config: Vec<u8>,
    worktrees: Vec<u8>,
    entries: Vec<FastForwardBranch>,
}
impl std::fmt::Debug for FastForwardPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastForwardPreview")
            .field("branches", &self.entries.len())
            .finish_non_exhaustive()
    }
}
impl FastForwardPreview {
    pub fn entries(&self) -> &[FastForwardBranch] {
        &self.entries
    }
}
#[derive(Debug)]
pub struct FastForwardReport {
    pub result: Result<()>,
    pub recovery: String,
    /// Fresh observation, including when interruption races a completed transaction.
    pub observed: Result<Vec<Branch>>,
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(format!("branch_fast_forward: {message}"))
}
impl Repository {
    fn ff_read(&self, arguments: &[&str], c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(arguments), vec![], c)?
            .success("branch-fast-forward")
    }
    pub fn preview_fast_forward_branches(
        &self,
        c: &CancellationToken,
    ) -> Result<FastForwardPreview> {
        self.runner
            .step("branch-fast-forward-preview", "review_fast_forward", || {
                self.require_idle(c)?;
                if std::fs::symlink_metadata(self.info().common_dir.join("info/grafts")).is_ok() {
                    return Err(invalid(
                        "Remove legacy grafts before reviewing real commit ancestry.",
                    ));
                }
                let config = self.ff_read(&["config", "--null", "--list"], c)?;
                let worktrees = self.ff_read(&["worktree", "list", "--porcelain", "-z"], c)?;
                let branches = self.branches(c)?;
                let checked = self.worktrees(c)?;
                let mut entries = Vec::new();
                for b in branches.iter().filter(|b| b.kind == BranchKind::Local) {
                    c.check()?;
                    if entries.len() == 1000 {
                        return Err(invalid("review at most 1,000 local branches at a time."));
                    }
                    let upstream = b.upstream.clone();
                    let target = upstream.as_ref().and_then(|u| {
                        branches
                            .iter()
                            .find(|other| &other.reference == u && other.symbolic_target.is_none())
                    });
                    let mut blocked = if b.symbolic_target.is_some() {
                        Some("symbolic branch")
                    } else if b.is_head
                        || b.checked_out_at.is_some()
                        || checked
                            .iter()
                            .any(|w| w.branch.as_ref() == Some(&b.reference))
                    {
                        Some("checked out in a worktree")
                    } else if upstream.is_none() {
                        Some("no upstream")
                    } else if target.is_none() {
                        Some("upstream unavailable locally; fetch first")
                    } else if target.is_some_and(|t| t.target == b.target) {
                        Some("already up to date")
                    } else {
                        None
                    };
                    if blocked.is_none() {
                        let target = target.expect("checked upstream");
                        let output = self.runner.run(
                            self.cwd(),
                            &args(&[
                                "--no-replace-objects",
                                "merge-base",
                                "--is-ancestor",
                                b.target.as_str(),
                                target.target.as_str(),
                            ]),
                            vec![],
                            c,
                        )?;
                        match output.status.code() {
                            Some(0) => (),
                            Some(1) => {
                                blocked = Some("diverged or ahead; requires an individual review")
                            }
                            _ => {
                                output.success("check fast-forward ancestry")?;
                            }
                        }
                    }
                    entries.push(FastForwardBranch {
                        branch: b.reference.clone(),
                        before: b.target.clone(),
                        upstream,
                        after: target.map(|t| t.target.clone()),
                        blocked,
                    });
                }
                if config != self.ff_read(&["config", "--null", "--list"], c)?
                    || worktrees != self.ff_read(&["worktree", "list", "--porcelain", "-z"], c)?
                    || branches != self.branches(c)?
                {
                    return Err(invalid("repository changed during review; review again."));
                }
                Ok(FastForwardPreview {
                    repository: self.info().common_dir.clone(),
                    config,
                    worktrees,
                    entries,
                })
            })
    }
    pub fn fast_forward_branches(
        &self,
        preview: &FastForwardPreview,
        selected: &[Vec<u8>],
        c: &CancellationToken,
    ) -> Result<FastForwardReport> {
        let _permit = self.acquire_write(c)?;
        let entries = self
            .runner
            .step("branch-fast-forward", "revalidate_fast_forward", || {
                if selected.is_empty()
                    || selected.len() > 200
                    || selected.iter().collect::<BTreeSet<_>>().len() != selected.len()
                {
                    return Err(invalid("select 1–200 distinct eligible branches."));
                }
                let current = self.preview_fast_forward_branches(c)?;
                if preview.repository != current.repository
                    || preview.config != current.config
                    || preview.worktrees != current.worktrees
                {
                    return Err(invalid(
                        "configuration or worktrees changed; review again. No refs were changed.",
                    ));
                }
                let mut entries = Vec::new();
                for name in selected {
                    let old = preview
                        .entries
                        .iter()
                        .find(|e| &e.branch == name && e.blocked.is_none())
                        .ok_or_else(|| {
                            invalid("selected branch was not eligible in this review.")
                        })?;
                    if !current.entries.contains(old) {
                        return Err(invalid(
                            "branch or upstream changed; review again. No refs were changed.",
                        ));
                    }
                    entries.push(old.clone());
                }
                Ok(entries)
            })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("invalid system clock"))?
            .as_nanos();
        let recovery = format!("refs/rocket/fast-forward/{}-{stamp}", std::process::id());
        let mut input = b"start\noption no-deref\n".to_vec();
        let mut verified = BTreeSet::new();
        for (i, entry) in entries.iter().enumerate() {
            let after = entry.after.as_ref().expect("eligible target");
            input.extend_from_slice(
                format!("create {recovery}/{i} {}\nupdate ", entry.before).as_bytes(),
            );
            input.extend_from_slice(&entry.branch);
            input.extend_from_slice(format!(" {after} {}\n", entry.before).as_bytes());
            let upstream = entry.upstream.as_ref().expect("eligible upstream");
            if !selected.contains(upstream) && verified.insert(upstream) {
                input.extend_from_slice(b"verify ");
                input.extend_from_slice(upstream);
                input.extend_from_slice(format!(" {after}\n").as_bytes());
            }
        }
        input.extend_from_slice(b"prepare\ncommit\n");
        let result = self
            .runner
            .step("branch-fast-forward", "update_selected_branches", || {
                self.runner
                    .write(
                        self.cwd(),
                        &args(&[
                            "update-ref",
                            "--no-deref",
                            "-m",
                            "Rocket reviewed fast-forward",
                            "--stdin",
                        ]),
                        input,
                        c,
                    )?
                    .success("branch-fast-forward")
                    .map(|_| ())
            });
        let observed = self
            .runner
            .step("branch-fast-forward", "observe_fast_forward", || {
                self.branches(&CancellationToken::new())
            });
        Ok(FastForwardReport {
            result,
            recovery,
            observed,
        })
    }
}
