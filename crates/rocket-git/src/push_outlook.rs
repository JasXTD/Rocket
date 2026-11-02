//! What a push would send, judged from the remote-tracking branches of the last fetch.
use crate::{runner::args, *};

/// Most commits a push outlook lists; `ahead` still counts all of them.
pub const PUSH_OUTLOOK_COMMITS: usize = 20;

/// A local view of pushing a branch to a remote branch. Nothing is contacted: the
/// remote may have moved since the last fetch, and Git decides at push time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOutlook {
    /// The destination as last fetched; None when it is not on the remote yet.
    pub tracked: Option<ObjectId>,
    /// Commits the destination lacks, newest first, at most [`PUSH_OUTLOOK_COMMITS`].
    pub commits: Vec<Commit>,
    pub ahead: u64,
    /// Commits the destination has that the branch lacks; such a push is refused.
    pub behind: u64,
}

impl Repository {
    /// Read-only. Compares local `branch` with `remote`'s `destination` as last
    /// fetched. A destination that was never fetched is treated as new: the commits
    /// listed are those no remote-tracking branch of `remote` has.
    pub fn push_outlook(
        &self,
        branch: &[u8],
        remote: &[u8],
        destination: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PushOutlook> {
        self.runner.step("push-outlook", "read_push_outlook", || {
            for (what, name) in [("branch", branch), ("remote", remote), ("destination", destination)] {
                if name.is_empty()
                    || name.len() > 4096
                    || name.contains(&0)
                    || name.starts_with(b"-")
                {
                    return Err(Error::InvalidInput(format!("invalid {what} name")));
                }
            }
            let branches = self.branches(cancel)?;
            let mut local = b"refs/heads/".to_vec();
            local.extend_from_slice(branch);
            let tip = branches
                .iter()
                .find(|b| b.kind == BranchKind::Local && b.reference == local)
                .map(|b| b.target.clone())
                .ok_or_else(|| Error::InvalidInput("the branch no longer exists".into()))?;
            let mut prefix = b"refs/remotes/".to_vec();
            prefix.extend_from_slice(remote);
            prefix.push(b'/');
            let mut tracking = prefix.clone();
            tracking.extend_from_slice(destination);
            let tracked = branches
                .iter()
                .find(|b| b.kind == BranchKind::Remote && b.reference == tracking)
                .map(|b| b.target.clone());
            // What the remote already has: the destination, or for a new branch every
            // branch fetched from that remote.
            let have: Vec<ObjectId> = match &tracked {
                Some(id) => vec![id.clone()],
                None => branches
                    .iter()
                    .filter(|b| {
                        b.kind == BranchKind::Remote
                            && b.symbolic_target.is_none()
                            && b.reference.starts_with(&prefix)
                    })
                    .map(|b| b.target.clone())
                    .collect(),
            };
            // Only validated object IDs reach rev-list, through stdin.
            let mut exclude: Vec<u8> = Vec::new();
            for id in &have {
                exclude.extend_from_slice(format!("^{id}\n").as_bytes());
            }
            let mut input = format!("{tip}\n").into_bytes();
            input.extend_from_slice(&exclude);
            let ahead = self.outlook_count(input.clone(), cancel)?;
            let behind = match &tracked {
                Some(id) => self.outlook_count(format!("{id}\n^{tip}\n").into_bytes(), cancel)?,
                None => 0,
            };
            let commits = if ahead == 0 {
                Vec::new()
            } else {
                let output = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&[
                            "rev-list",
                            "--topo-order",
                            &format!("--max-count={PUSH_OUTLOOK_COMMITS}"),
                            "--stdin",
                            "--",
                        ]),
                        input,
                        cancel,
                    )?
                    .success("push outlook")?;
                let ids = output
                    .split(|&b| b == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| parse::oid(line, "push outlook"))
                    .collect::<Result<Vec<_>>>()?;
                let objects: Vec<u8> = ids
                    .iter()
                    .flat_map(|id| id.as_str().bytes().chain(std::iter::once(b'\n')))
                    .collect();
                let output = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&["cat-file", "--batch", "--buffer"]),
                        objects,
                        cancel,
                    )?
                    .success("push outlook")?;
                parse::commits(&output, &ids)?
            };
            cancel.check()?;
            Ok(PushOutlook {
                tracked,
                commits,
                ahead,
                behind,
            })
        })
    }

    fn outlook_count(&self, input: Vec<u8>, cancel: &CancellationToken) -> Result<u64> {
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-list", "--count", "--stdin", "--"]),
                input,
                cancel,
            )?
            .success("push outlook")?;
        std::str::from_utf8(parse::line(&output))
            .ok()
            .and_then(|count| count.parse().ok())
            .ok_or_else(|| Error::parse("push outlook", "invalid commit count"))
    }
}
