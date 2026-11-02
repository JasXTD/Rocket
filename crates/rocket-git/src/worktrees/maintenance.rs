//! Selected-worktree maintenance; never run a repository-wide repair or prune.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorktreeMaintenance {
    Move,
    Repair,
    Prune,
}
impl WorktreeMaintenance {
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "worktree-move",
            Self::Repair => "worktree-repair",
            Self::Prune => "worktree-prune",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Move => "Move worktree",
            Self::Repair => "Repair worktree",
            Self::Prune => "Prune registration",
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Plan {
    pub kind: WorktreeMaintenance,
    admin: PathBuf,
    metadata: Vec<(PathBuf, Option<Vec<u8>>)>,
    gitfile: Option<Vec<u8>>,
}

impl Repository {
    pub fn preview_maintain_worktree(
        &self,
        kind: WorktreeMaintenance,
        entry: &WorktreeEntry,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.runner
            .step("worktree-preview", "review_worktree_maintenance", || {
                let (path, plan) = self.maintenance_context(kind, entry, destination, cancel)?;
                Ok(WorktreePreview {
                    repository: self.info().common_dir.clone(),
                    path,
                    target: entry
                        .head
                        .clone()
                        .ok_or(Error::Worktree(WorktreeFailure::TargetUnavailable))?,
                    checkout: None,
                    entry: Some(entry.clone()),
                    git_dir: Some(plan.admin.clone()),
                    maintenance: Some(plan),
                })
            })
    }

    fn maintenance_context(
        &self,
        kind: WorktreeMaintenance,
        e: &WorktreeEntry,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<(PathBuf, Plan)> {
        cancel.check()?;
        if e.repository != self.info().common_dir || !self.worktrees(cancel)?.contains(e) {
            return refused(WorktreeFailure::StalePreview);
        }
        if e.main || e.bare || self.info().worktree.as_ref() == Some(&e.path) {
            return refused(WorktreeFailure::ProtectedWorktree);
        }
        if e.locked.is_some() {
            return refused(WorktreeFailure::Locked);
        }
        let registrations = self.info().common_dir.join("worktrees");
        let _guard = init::guard_directories(&registrations)?;
        let mut found = None;
        for item in fs::read_dir(&registrations)
            .map_err(|e| Error::io("inspect worktree registrations", e))?
        {
            cancel.check()?;
            let admin = item
                .map_err(|e| Error::io("inspect worktree registration", e))?
                .path();
            local_files::directory(&admin)?;
            let bytes = local_files::read_config(&admin.join("gitdir"))?;
            if let Some(backlink) = absolute_link(&bytes, "")
                && equivalent_path(&backlink, &e.path.join(".git"))
            {
                if found.is_some() {
                    return refused(WorktreeFailure::StalePreview);
                }
                found = Some(admin);
            }
        }
        let admin = found.ok_or(Error::Worktree(WorktreeFailure::Missing))?;
        let common = local_files::read_config(&admin.join("commondir"))?;
        let common = std::str::from_utf8(&common)
            .map_err(|_| Error::Worktree(WorktreeFailure::StalePreview))?
            .trim_end_matches(['\r', '\n']);
        if admin.join(common).canonicalize().ok().as_ref() != Some(&self.info().common_dir) {
            return refused(WorktreeFailure::StalePreview);
        }
        let (path, gitfile) = match kind {
            WorktreeMaintenance::Move => {
                let repo = self.removal_context(e, cancel)?;
                if repo.info().git_dir != admin {
                    return refused(WorktreeFailure::StalePreview);
                }
                let path = self.worktree_destination(destination, cancel)?;
                if e.path.components().next() != path.components().next() {
                    return Err(Error::Unsupported("Moving a worktree between drives is not supported. Choose a new folder on the same drive.".into()));
                }
                PreparedLink::capture(&admin, &e.path)?;
                (path, Some(local_files::read_config(&e.path.join(".git"))?))
            }
            WorktreeMaintenance::Repair => {
                if !path_missing(&e.path) {
                    return refused(WorktreeFailure::DestinationExists);
                }
                let destination = init::destination_path(destination)
                    .map_err(|_| Error::Worktree(WorktreeFailure::InvalidDestination))?;
                let path = destination
                    .canonicalize()
                    .map_err(|e| Error::io("inspect moved worktree", e))?;
                let _guard = init::guard_directories(&path)?;
                if path.starts_with(&self.info().common_dir)
                    || self
                        .worktrees(cancel)?
                        .iter()
                        .any(|other| equivalent_path(&other.path, &path))
                {
                    return refused(WorktreeFailure::InvalidDestination);
                }
                let gitfile = local_files::read_config(&path.join(".git"))?;
                if absolute_link(&gitfile, "gitdir: ")
                    .and_then(|p| p.canonicalize().ok())
                    .as_ref()
                    != Some(&admin)
                {
                    return refused(WorktreeFailure::StalePreview);
                }
                let mut moved = e.clone();
                moved.path = path.clone();
                let repo = self.linked_repository(&moved, cancel)?;
                if !repo.operation_state(cancel)?.is_idle() {
                    return refused(WorktreeFailure::ActiveOperation);
                }
                (path, Some(gitfile))
            }
            WorktreeMaintenance::Prune => {
                if !path_missing(&e.path) {
                    return refused(WorktreeFailure::DestinationExists);
                }
                // A disconnected drive or missing parent is not evidence of removal.
                let _parent = init::guard_directories(
                    e.path
                        .parent()
                        .ok_or(Error::Worktree(WorktreeFailure::InvalidDestination))?,
                )?;
                (e.path.clone(), None)
            }
        };
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "BISECT_START",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
        ] {
            if !path_missing(&admin.join(name)) {
                return refused(WorktreeFailure::ActiveOperation);
            }
        }
        let metadata = snapshot_metadata(&admin, cancel)?;
        Ok((
            path,
            Plan {
                kind,
                admin,
                metadata,
                gitfile,
            },
        ))
    }

    pub(super) fn maintain_worktree(
        &self,
        p: &WorktreePreview,
        cancel: &CancellationToken,
    ) -> Result<WorktreeWriteReport> {
        let plan = p.maintenance.as_ref().expect("maintenance preview");
        let entry = p.entry.as_ref().expect("maintenance entry");
        let backlink = plan
            .metadata
            .iter()
            .find_map(|(path, bytes)| {
                (path == Path::new("gitdir"))
                    .then_some(bytes.as_ref())
                    .flatten()
            })
            .ok_or(Error::Worktree(WorktreeFailure::StalePreview))?;
        let revalidate = || -> Result<()> {
            if p.repository != self.info().common_dir {
                return refused(WorktreeFailure::StalePreview);
            }
            let (path, fresh) = self.maintenance_context(plan.kind, entry, &p.path, cancel)?;
            if path != p.path
                || fresh.admin != plan.admin
                || fresh.metadata != plan.metadata
                || fresh.gitfile != plan.gitfile
            {
                return refused(WorktreeFailure::StalePreview);
            }
            Ok(())
        };
        self.runner
            .step(p.label(), "revalidate_worktree", revalidate)?;
        let mut backup = None;
        let mut changed = false;
        let command = self.runner.observe("worktree_command", p.label(), || {
            if let Some(reference) = p.recovery_reference() {
                self.runner.step(p.label(), "retain_worktree_commit", || {
                    self.retain_worktree_commit(&reference, &p.target, cancel)
                })?;
            }
            // Backups live outside worktrees/ so Git does not treat them as registrations.
            let _common_guard = init::guard_directories(&self.info().common_dir)?;
            let workspace = allocate(&self.info().common_dir, cancel)?;
            backup = Some(workspace.clone());
            let _backup_guard = init::guard_directories(&workspace)?;
            let _parent_guard =
                init::guard_directories(plan.admin.parent().expect("admin parent"))?;
            self.runner
                .step(p.label(), "backup_worktree_metadata", || {
                    let mut note = fs::OpenOptions::new().write(true).create_new(true)
                        .open(workspace.join("recovery.txt")).map_err(|e| Error::io("create worktree recovery note", e))?;
                    write!(note, "ROCKET-WORKTREE-RECOVERY-1\nOperation: {}\nOriginal registration: {}\nOriginal worktree: {}\nReviewed destination: {}\nCommit: {}\n\nMetadata only; working files are not backed up here. This record describes the reviewed operation, not proof it completed. Inspect the current worktree list and paths before any manual recovery. Never overwrite an existing registration.\n",
                        p.label(), plan.admin.display(), entry.path.display(), p.path.display(), p.target)
                        .and_then(|_| note.sync_all()).map_err(|e| Error::io("save worktree recovery note", e))?;
                    if plan.kind != WorktreeMaintenance::Prune {
                        let original = local_files::read_config(&plan.admin.join("gitdir"))?;
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(workspace.join("gitdir.before"))
                            .map_err(|e| Error::io("create worktree metadata backup", e))?;
                        file.write_all(&original)
                            .and_then(|_| file.sync_all())
                            .map_err(|e| Error::io("save worktree metadata backup", e))?;
                    }
                    Ok(())
                })?;
            // Recovery-ref hooks and backup I/O may allow outside changes. Check again.
            revalidate()?;
            self.runner
                .step(p.label(), "publish_worktree_maintenance", || {
                    cancel.check()?;
                    let mut locks = MetadataLocks::acquire(&plan.admin)?;
                    if snapshot_metadata_inner(&plan.admin, cancel, Some(&locks))? != plan.metadata
                    {
                        return refused(WorktreeFailure::StalePreview);
                    }
                    match plan.kind {
                        WorktreeMaintenance::Move => {
                            let _from = init::guard_directories(
                                entry.path.parent().expect("worktree parent"),
                            )?;
                            let _to = init::guard_directories(
                                p.path.parent().expect("destination parent"),
                            )?;
                            let link = PreparedLink::capture(&plan.admin, &entry.path)?;
                            if Some(&link.gitfile) != plan.gitfile.as_ref() || &link.backlink != backlink {
                                return refused(WorktreeFailure::StalePreview);
                            }
                            // Windows rename is same-volume and non-replacing. No copy/delete fallback.
                            local_files::move_path(&entry.path, &p.path, false)?;
                            changed = true;
                            link.publish(&p.path)?;
                        }
                        WorktreeMaintenance::Repair => {
                            let _to = init::guard_directories(&p.path)?;
                            let link = PreparedLink {
                                admin: plan.admin.clone(),
                                gitfile: plan.gitfile.clone().expect("repair gitfile"),
                                backlink: backlink.clone(),
                                _guard: init::guard_directories(&plan.admin)?,
                            };
                            link.publish(&p.path)?;
                            changed = true;
                        }
                        WorktreeMaintenance::Prune => {
                            let _source_parent = init::guard_directories(entry.path.parent().expect("registered worktree parent"))?;
                            if !path_missing(&entry.path) { return refused(WorktreeFailure::StalePreview); }
                            // Keep the entire administrative directory (index, refs, reflogs, config).
                            // Never recursively remove it or prune another registration.
                            local_files::move_path(
                                &plan.admin,
                                &workspace.join("registration"),
                                false,
                            )?;
                            locks.directory = workspace.join("registration");
                            changed = true;
                        }
                    }
                    // Once a folder moved, finish its minimal backlink update even if cancelled.
                    cancel.check()?;
                    Ok(WriteOutput {
                        stdout: vec![],
                        stderr: vec![],
                    })
                })
        });
        let fresh = CancellationToken::new();
        let observed = self
            .runner
            .step(p.label(), "observe_worktrees", || self.worktrees(&fresh));
        let retained = if let Some(reference) = p.recovery_reference() {
            // Cancellation may arrive after update-ref committed but before it returned.
            matches!(self.worktree_recovery_ref(&reference, &fresh), Ok(Some((id, false))) if id == p.target).then_some(reference)
        } else {
            None
        };
        let verification = self.runner.step(p.label(), "verify_worktree_result", || {
            if !changed {
                return refused(WorktreeFailure::VerificationFailed);
            }
            let entries = observed
                .as_ref()
                .map_err(|_| Error::Worktree(WorktreeFailure::VerificationFailed))?;
            if plan.kind == WorktreeMaintenance::Prune {
                if retained.is_none()
                    || !path_missing(&entry.path)
                    || !path_missing(&plan.admin)
                    || entries
                        .iter()
                        .any(|e| equivalent_path(&e.path, &entry.path))
                {
                    return refused(WorktreeFailure::VerificationFailed);
                }
                let archived = backup.as_ref().expect("backup").join("registration");
                if snapshot_metadata(&archived, &fresh)? != plan.metadata {
                    return refused(WorktreeFailure::VerificationFailed);
                }
            } else {
                let e = entries
                    .iter()
                    .find(|e| equivalent_path(&e.path, &p.path))
                    .ok_or(Error::Worktree(WorktreeFailure::VerificationFailed))?;
                if e.head != entry.head
                    || e.branch != entry.branch
                    || e.prunable.is_some()
                    || !path_missing(&entry.path)
                {
                    return refused(WorktreeFailure::VerificationFailed);
                }
                let repo = self.linked_repository(e, &fresh)?;
                if repo.info().git_dir != plan.admin {
                    return refused(WorktreeFailure::VerificationFailed);
                }
                PreparedLink::capture(&plan.admin, &p.path)?;
            }
            Ok(())
        });
        let mut report = self.worktree_report(p, command, observed, verification, None, retained);
        report.metadata_backup = backup;
        Ok(report)
    }
}

fn absolute_link(bytes: &[u8], prefix: &str) -> Option<PathBuf> {
    let path = PathBuf::from(
        std::str::from_utf8(bytes)
            .ok()?
            .trim_end_matches(['\r', '\n'])
            .strip_prefix(prefix)?,
    );
    path.is_absolute().then_some(path)
}

/// Exact bounded snapshots avoid metadata-only fingerprints missing quick external edits.
fn snapshot_metadata(
    root: &Path,
    cancel: &CancellationToken,
) -> Result<Vec<(PathBuf, Option<Vec<u8>>)>> {
    snapshot_metadata_inner(root, cancel, None)
}
fn snapshot_metadata_inner(
    root: &Path,
    cancel: &CancellationToken,
    own: Option<&MetadataLocks>,
) -> Result<Vec<(PathBuf, Option<Vec<u8>>)>> {
    let _guard = init::guard_directories(root)?;
    let mut stack = vec![root.to_owned()];
    let mut records = vec![];
    let mut total = 0;
    while let Some(dir) = stack.pop() {
        cancel.check()?;
        for item in fs::read_dir(&dir).map_err(|e| Error::io("review worktree metadata", e))? {
            cancel.check()?;
            let path = item
                .map_err(|e| Error::io("review worktree metadata", e))?
                .path();
            let name = path.file_name().expect("metadata file").to_string_lossy();
            if name == "locked" || name.ends_with(".lock") {
                if own.is_some_and(|l| l.owns(&path)) {
                    continue;
                }
                return refused(WorktreeFailure::Locked);
            }
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| Error::io("inspect worktree metadata", e))?;
            if local_files::redirected(&meta) {
                return refused(WorktreeFailure::UnsupportedIndex);
            }
            if meta.is_file() && meta.len() > 2 * 1024 * 1024 {
                return Err(Error::Unsupported(
                    "A worktree metadata file exceeds the 2 MiB review limit; inspect it with Git"
                        .into(),
                ));
            }
            let value = if meta.is_dir() {
                stack.push(path.clone());
                None
            } else {
                Some(local_files::read_config(&path)?)
            };
            total += value.as_ref().map_or(0, Vec::len);
            records.push((
                path.strip_prefix(root)
                    .expect("metadata descendant")
                    .to_owned(),
                value,
            ));
            if records.len() > 4096 || total > 16 * 1024 * 1024 {
                return Err(Error::Unsupported("Worktree maintenance review exceeds 4096 metadata entries or 16 MiB; inspect it with Git".into()));
            }
        }
    }
    records.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(records)
}

// Cooperate with Git's worktree, index, HEAD and config locks. Never remove an
// existing lock, or one replaced by another process; a partial acquisition unwinds.
struct MetadataLocks {
    directory: PathBuf,
    names: Vec<&'static str>,
    value: Vec<u8>,
}
impl MetadataLocks {
    fn acquire(directory: &Path) -> Result<Self> {
        let value = format!(
            "Rocket maintenance {} {} {:?}\n",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
        )
        .into_bytes();
        let mut locks = Self {
            directory: directory.to_owned(),
            names: vec![],
            value,
        };
        for name in ["locked", "index.lock", "HEAD.lock", "config.worktree.lock"] {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(name))
                .map_err(|e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        Error::Worktree(WorktreeFailure::Locked)
                    } else {
                        Error::io("lock worktree maintenance", e)
                    }
                })?;
            file.write_all(&locks.value)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io("save worktree maintenance lock", e))?;
            locks.names.push(name);
        }
        Ok(locks)
    }
    fn owns(&self, path: &Path) -> bool {
        path.parent() == Some(self.directory.as_path())
            && self
                .names
                .iter()
                .any(|name| path.file_name() == Some(std::ffi::OsStr::new(name)))
            && local_files::read_config(path).is_ok_and(|b| b == self.value)
    }
}
impl Drop for MetadataLocks {
    fn drop(&mut self) {
        for name in &self.names {
            let path = self.directory.join(name);
            if self.owns(&path) {
                let _ = fs::remove_file(path);
            }
        }
    }
}
