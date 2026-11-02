//! What each worktree holds now: its uncommitted changes and its last commit.
use crate::{runner::args, *};
use std::path::PathBuf;

/// One worktree's working folder as read now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeStatus {
    pub path: PathBuf,
    /// Changed, staged, conflicted and new files; ignored ones are not counted.
    pub changes: usize,
    /// The checked-out commit's subject and author time; None before a first commit.
    pub last: Option<(String, i64)>,
}

impl Repository {
    /// Read-only. For each listed worktree with a working folder on disk, how many
    /// files differ from its commit, and that commit. Bare entries and folders
    /// that are gone are left out.
    pub fn worktree_statuses(
        &self,
        entries: &[WorktreeEntry],
        cancel: &CancellationToken,
    ) -> Result<Vec<WorktreeStatus>> {
        self.runner.step("worktrees", "read_worktree_status", || {
            let mut statuses = Vec::new();
            for entry in entries.iter().filter(|e| !e.bare && e.prunable.is_none()) {
                cancel.check()?;
                if !entry.path.is_dir() {
                    continue;
                }
                let output = self
                    .runner
                    .run(
                        &entry.path,
                        &args(&[
                            "status",
                            "--porcelain=v2",
                            "-z",
                            "--branch",
                            "--untracked-files=all",
                            "--ignore-submodules=none",
                            "--renames",
                        ]),
                        Vec::new(),
                        cancel,
                    )?
                    .success("worktree status")?;
                let changes = parse::status(&output)?
                    .entries
                    .iter()
                    .filter(|e| e.kind != EntryKind::Ignored)
                    .count();
                let last = match &entry.head {
                    Some(head) => {
                        let output = self
                            .runner
                            .run(
                                &entry.path,
                                &args(&["log", "-1", "--no-show-signature", "--format=%at%x00%s", head.as_str(), "--"]),
                                Vec::new(),
                                cancel,
                            )?
                            .success("worktree status")?;
                        let line = parse::line(&output);
                        line.iter().position(|b| *b == 0).and_then(|split| {
                            let time = std::str::from_utf8(&line[..split]).ok()?.parse().ok()?;
                            Some((String::from_utf8_lossy(&line[split + 1..]).into_owned(), time))
                        })
                    }
                    None => None,
                };
                statuses.push(WorktreeStatus {
                    path: entry.path.clone(),
                    changes,
                    last,
                });
            }
            Ok(statuses)
        })
    }
}
