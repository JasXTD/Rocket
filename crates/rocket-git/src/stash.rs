use crate::{model::bytes_to_os, parse, runner::args, *};
use std::ffi::OsString;

impl Repository {
    /// Read a stash by immutable object ID, even if its reflog position changes.
    /// This does not apply changes or consult external diff/textconv commands.
    pub fn stash_diff(
        &self,
        entry: &StashEntry,
        part: StashPart,
        cancel: &CancellationToken,
    ) -> Result<Diff> {
        let Some(target) = self.stash_diff_target(entry, part, cancel)? else {
            return Ok(Diff { patch: Vec::new() });
        };
        self.diff(&DiffOptions { target, ..Default::default() }, cancel)
    }

    pub(crate) fn stash_diff_target(&self, entry: &StashEntry, part: StashPart, cancel: &CancellationToken) -> Result<Option<DiffTarget>> {
        if entry.repository != self.info().common_dir {
            return Err(Error::InvalidInput(
                "stash entry belongs to another repository".into(),
            ));
        }
        let page = self.history(HistoryStart::Commit(entry.id.clone()), 1, cancel)?;
        let commit = page
            .commits
            .first()
            .filter(|c| c.id == entry.id && (2..=3).contains(&c.parents.len()))
            .ok_or_else(|| {
                Error::InvalidInput("selected object is not a supported stash".into())
            })?;
        let target = match part {
            StashPart::Tracked => DiffTarget::Between {
                base: commit.parents[0].to_string(),
                target: entry.id.to_string(),
            },
            StashPart::Index => DiffTarget::Between {
                base: commit.parents[0].to_string(),
                target: commit.parents[1].to_string(),
            },
            StashPart::Untracked => match commit.parents.get(2) {
                Some(id) => DiffTarget::Commit { id: id.clone() },
                None => return Ok(None),
            },
        };
        Ok(Some(target))
    }
    /// List the shared stash reflog, newest first, preserving subject bytes.
    /// Git keeps stashes with a working folder and refuses `git stash` in a bare
    /// repository.
    pub fn stashes(&self, cancel: &CancellationToken) -> Result<Vec<StashEntry>> {
        if self.info().worktree.is_none() {
            return Err(Error::WorktreeRequired);
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "stash",
                    "list",
                    "-z",
                    "--format=%H%x00%gs",
                    "--no-color",
                    "--no-decorate",
                    "--no-show-signature",
                    "--no-patch",
                    "--encoding=none",
                ]),
                Vec::new(),
                cancel,
            )?
            .success("list stashes")?;
        let entries = parse_stashes(&output, &self.info().common_dir)?;
        cancel.check()?;
        Ok(entries)
    }

    /// Save all tracked changes, optionally including untracked (never ignored)
    /// files. Git stores the stash before cleaning the saved worktree changes.
    /// Dirty submodules are rejected: their working files cannot be saved here.
    pub fn push_stash(
        &self,
        message: Option<&str>,
        include_untracked: bool,
        cancel: &CancellationToken,
    ) -> Result<StashWriteReport> {
        self.push_stash_paths(message, include_untracked, &[], cancel)
    }

    /// Like [`Repository::push_stash`], but save only the changes of `paths` when it
    /// is not empty; other (unstaged or untracked) changes stay in the working tree.
    /// Each path must be changed in the current status and pass
    /// [`StatusEntry::stashable_alone`]; an untracked path needs `include_untracked`.
    /// Every staged change must be among `paths`, because Git records the whole
    /// index in a stash. Paths are literal, never patterns.
    pub fn push_stash_paths(
        &self,
        message: Option<&str>,
        include_untracked: bool,
        paths: &[RepoPath],
        cancel: &CancellationToken,
    ) -> Result<StashWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.push_stash_paths_locked(message, include_untracked, paths, cancel)
    }
    pub(crate) fn push_stash_paths_locked(&self, message: Option<&str>, include_untracked: bool, paths: &[RepoPath], cancel: &CancellationToken) -> Result<StashWriteReport> {
        if message.is_some_and(|m| m.trim().is_empty() || m.contains('\0') || m.len() > 4096) {
            return Err(Error::InvalidInput(
                "stash message must be nonblank, contain no NUL, and fit in 4096 bytes".into(),
            ));
        }
        self.require_worktree(cancel)?;
        self.require_idle(cancel)?;
        let status = self.status(cancel)?;
        if status
            .entries
            .iter()
            .any(|e| e.kind == EntryKind::Unmerged || e.submodule.is_some())
        {
            return Err(Error::InvalidInput(
                "resolve conflicts and handle changed submodules before saving a stash".into(),
            ));
        }
        self.check_stash_replacements(&status, cancel)?;
        let mut pathspecs = Vec::new();
        for path in paths {
            cancel.check()?;
            let entry = status
                .entries
                .iter()
                .find(|e| &e.path == path)
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "a selected file has no changes to stash; refresh and select again".into(),
                    )
                })?;
            if entry.kind == EntryKind::Untracked && !include_untracked {
                return Err(Error::InvalidInput(
                    "include untracked files to stash a selected untracked file".into(),
                ));
            }
            if !entry.stashable_alone() {
                return Err(Error::InvalidInput(
                    "Git cannot stash a staged deletion or rename by itself; stash all changes instead".into(),
                ));
            }
            pathspecs.push(bytes_to_os(
                &[b":(top,literal)".as_slice(), entry.path.as_bytes()].concat(),
            )?);
        }
        // Even when it saves only some paths, Git records the whole index in the
        // stash: an unselected staged change would be saved (and reapplied later)
        // while also staying staged here. So every staged change must be selected.
        if let Some(entry) = status
            .entries
            .iter()
            .find(|e| !paths.is_empty() && e.staged() && !paths.contains(&e.path))
        {
            return Err(Error::InvalidInput(if entry.stashable_alone() {
                "a stash always saves every staged change; select all staged files or unstage the others first".into()
            } else {
                "Git cannot stash a staged deletion or rename by itself; unstage it first or stash all changes".into()
            }));
        }
        let mut arguments = args(&[
            // Git stash internally invokes clean with its own `:/` pathspec, so
            // magic stays on; selected paths carry their own `literal` magic.
            "--no-literal-pathspecs",
            "-c",
            "submodule.recurse=false",
            "stash",
            "push",
            "--no-keep-index",
            if include_untracked {
                "--include-untracked"
            } else {
                "--no-include-untracked"
            },
        ]);
        if let Some(message) = message {
            arguments.extend(args(&["--message", message]));
        }
        arguments.push("--".into());
        arguments.extend(pathspecs);
        self.execute_stash_write(WriteOperation::PushStash, &arguments, cancel)
    }

    /// Apply a pinned stash object, retaining its stash-list entry even on success.
    /// `restore_index` requests the saved staging split; false uses Git's default
    /// apply behavior (newly added files can still be staged).
    /// Conflicts are reported by Git and remain in the index for manual resolution.
    pub fn apply_stash(
        &self,
        id: &ObjectId,
        restore_index: bool,
        cancel: &CancellationToken,
    ) -> Result<StashWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.apply_stash_locked(id, restore_index, cancel)
    }
    pub(crate) fn apply_stash_locked(&self, id: &ObjectId, restore_index: bool, cancel: &CancellationToken) -> Result<StashWriteReport> {
        self.require_worktree(cancel)?;
        self.require_idle(cancel)?;
        self.check_stash_ignored_collisions(id, cancel)?;
        let mut arguments = args(&[
            "-c",
            "submodule.recurse=false",
            "-c",
            "stash.index=false",
            "-c",
            "rerere.autoupdate=false",
            "stash",
            "apply",
        ]);
        if restore_index {
            arguments.push("--index".into());
        }
        arguments.push(id.as_str().into());
        self.execute_stash_write(WriteOperation::ApplyStash, &arguments, cancel)
    }

    /// Drop exactly the observed entry if its index and ID still match. A changed
    /// list is rejected, never silently retargeted. Git has no atomic compare-and-
    /// drop API: another application's reflog edit can still race this precheck.
    /// Coordinate external stash writers before calling this destructive operation.
    pub fn drop_stash(
        &self,
        entry: &StashEntry,
        cancel: &CancellationToken,
    ) -> Result<StashWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.drop_stash_locked(entry, cancel)
    }
    pub(crate) fn drop_stash_locked(&self, entry: &StashEntry, cancel: &CancellationToken) -> Result<StashWriteReport> {
        self.require_worktree(cancel)?;
        if entry.repository != self.info().common_dir {
            return Err(Error::InvalidInput(
                "stash entry belongs to another repository".into(),
            ));
        }
        let entries = self.stashes(cancel)?;
        if entries.get(entry.index) != Some(entry) {
            return Err(Error::InvalidInput(
                "stash list changed; refresh and select the entry again".into(),
            ));
        }
        self.execute_stash_write(
            WriteOperation::DropStash,
            &args(&["stash", "drop", &format!("stash@{{{}}}", entry.index)]),
            cancel,
        )
    }

    fn execute_stash_write(
        &self,
        operation: WriteOperation,
        arguments: &[OsString],
        cancel: &CancellationToken,
    ) -> Result<StashWriteReport> {
        let write = self.execute_write(operation, arguments, Vec::new(), cancel)?;
        let stashes = self.runner.observe("refresh", "stashes", || {
            self.stashes(&CancellationToken::new())
        });
        Ok(StashWriteReport { write, stashes })
    }

    fn check_stash_replacements(&self, status: &Status, cancel: &CancellationToken) -> Result<()> {
        // Stash push internally resets to HEAD. Git can overwrite an untracked
        // replacement of a staged deletion, rename source, or file->directory
        // change without including that replacement in the saved tracked tree.
        for entry in &status.entries {
            cancel.check()?;
            if matches!(entry.kind, EntryKind::Untracked | EntryKind::Ignored) {
                continue;
            }
            let mut paths = vec![(&entry.path, entry.index == Change::Deleted)];
            if (entry.index == Change::Renamed || entry.worktree == Change::Renamed)
                && let Some(original) = &entry.original_path
            {
                paths.push((original, true));
            }
            for (path, replacement) in paths {
                match std::fs::symlink_metadata(self.cwd().join(path.argument()?)) {
                    Ok(metadata) if replacement || metadata.is_dir() => return Err(Error::InvalidInput(
                        "move aside untracked/ignored replacements of tracked paths before stashing".into(),
                    )),
                    Ok(_) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(Error::io("check stash replacement", error)),
                }
            }
        }
        Ok(())
    }

    fn check_stash_ignored_collisions(
        &self,
        id: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<()> {
        // Stash apply lacks merge's --no-overwrite-ignore switch. Ask Git which
        // incoming paths (and parents) are ignored, then reject existing collisions.
        // --no-renames includes both sides so replaced directories are covered too.
        let changed = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "diff-tree",
                    "--no-commit-id",
                    "--name-only",
                    "--no-renames",
                    "--no-relative",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=none",
                    "-r",
                    "-z",
                    &format!("{id}^1"),
                    id.as_str(),
                    "--",
                ]),
                Vec::new(),
                cancel,
            )?
            .success("check stash paths")?;
        let mut seen = std::collections::HashSet::new();
        let mut input = Vec::new();
        for path in nul_paths(&changed)? {
            cancel.check()?;
            let relative = RepoPath::new(path.to_vec())?;
            // Git can replace an entire directory with a saved file, silently
            // removing ignored descendants even when the directory isn't ignored.
            // Require the caller to move that directory aside first.
            match std::fs::symlink_metadata(self.cwd().join(relative.argument()?)) {
                Ok(metadata) if metadata.is_dir() => return Err(Error::InvalidInput(
                    "move aside directories that collide with saved stash paths before applying it"
                        .into(),
                )),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io("check stash path collision", error)),
            }
            let mut candidate = path;
            loop {
                cancel.check()?;
                if seen.insert(candidate.to_vec()) {
                    if candidate.len().saturating_add(3)
                        > (8 * 1024 * 1024usize).saturating_sub(input.len())
                    {
                        return Err(Error::InvalidInput(
                            "stash collision check exceeds 8 MiB".into(),
                        ));
                    }
                    // check-ignore takes pathnames, not pathspec magic. Prefix
                    // ./ so Unix names beginning with ':' remain ordinary paths.
                    input.extend_from_slice(b"./");
                    input.extend_from_slice(candidate);
                    input.push(0);
                }
                match candidate.iter().rposition(|b| *b == b'/') {
                    Some(end) => candidate = &candidate[..end],
                    None => break,
                }
            }
        }
        if input.is_empty() {
            return Ok(());
        }
        let ignored = self.runner.run(
            self.cwd(),
            &args(&["--no-literal-pathspecs", "check-ignore", "-z", "--stdin"]),
            input,
            cancel,
        )?;
        if ignored.status.code() == Some(1)
            && ignored.stdout.is_empty()
            && ignored.stderr.is_empty()
        {
            return Ok(());
        }
        let ignored = ignored.success("check ignored stash paths")?;
        for path in nul_paths(&ignored)? {
            cancel.check()?;
            let path = RepoPath::new(path.strip_prefix(b"./").unwrap_or(path).to_vec())?;
            match std::fs::symlink_metadata(self.cwd().join(path.argument()?)) {
                Ok(_) => {
                    return Err(Error::InvalidInput(
                        "move aside ignored files that collide with this stash before applying it"
                            .into(),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io("check ignored stash collision", error)),
            }
        }
        Ok(())
    }
}

fn nul_paths(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = bytes
        .strip_suffix(&[0])
        .ok_or_else(|| Error::parse("stash paths", "unterminated path"))?;
    let paths: Vec<_> = bytes.split(|b| *b == 0).collect();
    if paths.iter().any(|p| p.is_empty()) {
        return Err(Error::parse("stash paths", "empty path"));
    }
    Ok(paths)
}

fn parse_stashes(bytes: &[u8], repository: &std::path::Path) -> Result<Vec<StashEntry>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = bytes
        .strip_suffix(&[0])
        .ok_or_else(|| Error::parse("list stashes", "unterminated record"))?;
    let mut fields = bytes.split(|b| *b == 0);
    let mut entries = Vec::new();
    while let Some(id) = fields.next() {
        let subject = fields
            .next()
            .ok_or_else(|| Error::parse("list stashes", "missing subject"))?;
        entries.push(StashEntry {
            index: entries.len(),
            id: parse::oid(id, "list stashes")?,
            subject: subject.to_vec(),
            repository: repository.to_owned(),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stash_records_preserve_bytes_and_reject_truncation() {
        let path = std::path::Path::new("repo");
        let mut record = b"0123456789012345678901234567890123456789\0raw \xff\0".to_vec();
        let entry = parse_stashes(&record, path).unwrap().remove(0);
        assert_eq!(entry.subject, b"raw \xff");
        record.pop();
        assert!(parse_stashes(&record, path).is_err());
        assert!(parse_stashes(b"bad\0subject\0", path).is_err());
        assert!(parse_stashes(b"0123456789012345678901234567890123456789\0", path).is_err());
    }
}
