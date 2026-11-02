//! Relocate a clean checkout without renaming its retained administration folder.
//! Everything is prepared and backed up first. Once the directory moves, finish
//! the short filesystem publication even if cancellation arrives; no Git process
//! runs in that section. Partial failures retain both the checkout and metadata.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct MovePlan {
    pub destination: PathBuf,
    links: Vec<Link>,
}
#[derive(Clone, PartialEq, Eq)]
struct Link {
    directory: PathBuf,
    destination: PathBuf,
    admin: PathBuf,
    gitfile: FileState,
    config: FileState,
}
fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io("inspect submodule move destination", e)),
        Ok(_) => fail(SubmoduleFailure::DestinationOccupied),
    }
}
impl Repository {
    pub(super) fn plan_submodule_move(
        &self,
        p: &SubmoduleActionPreview,
        destination: &RepoPath,
        c: &CancellationToken,
    ) -> Result<MovePlan> {
        let dest = self.submodule_path(destination.as_bytes())?;
        absent(&dest)?;
        local_files::directory(&p.directory)?;
        let parent = dest
            .parent()
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
        if !parent.exists() {
            return Err(Error::InvalidInput("Create the destination's parent folder first, then review the submodule move again.".into()));
        }
        let _pin = local_files::pin_directories(self.cwd(), parent)?;
        let resolved = parent
            .canonicalize()
            .map_err(|e| Error::io("resolve submodule move destination", e))?;
        if resolved != parent
            || !resolved.starts_with(self.cwd())
            || resolved.starts_with(&self.info().common_dir)
            || resolved.starts_with(&self.info().git_dir)
            || dest.starts_with(&p.directory)
            || p.directory.starts_with(&dest)
        {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let mut ancestor = Some(parent);
        while let Some(dir) = ancestor.filter(|d| *d != self.cwd()) {
            if dir.join(".git").exists() {
                return fail(SubmoduleFailure::InvalidLayout);
            }
            ancestor = dir.parent();
        }
        let mut a = args(&["ls-files", "-z", "--"]);
        a.push(destination.argument()?);
        if !self
            .runner
            .run(self.cwd(), &a, vec![], c)?
            .success("submodule-move-index")?
            .is_empty()
        {
            return fail(SubmoduleFailure::DestinationOccupied);
        }
        let mut links = Vec::new();
        for e in p
            .entries
            .iter()
            .filter(|e| e.directory.starts_with(&p.directory) && e.initialized)
        {
            c.check()?;
            let child = self.client().open(&e.directory, c)?;
            let admin = &child.info().git_dir;
            // Embedded .git directories and linked worktrees require a different
            // relocation scheme. Refuse them rather than silently breaking links.
            if admin != &child.info().common_dir
                || !admin.starts_with(self.info().common_dir.join("modules"))
            {
                return Err(Error::InvalidInput("Move supports initialized submodules with a gitfile and retained module metadata. Convert embedded .git directories with Git before moving.".into()));
            }
            let gitfile = FileState::read(e.directory.join(".git"))?;
            let config = FileState::read(admin.join("config"))?;
            if gitfile.bytes.is_none() || config.bytes.is_none() {
                return fail(SubmoduleFailure::InvalidLayout);
            }
            child.submodule_local_section("core.worktree", &config.path, c)?;
            links.push(Link {
                directory: e.directory.clone(),
                destination: dest.join(
                    e.directory
                        .strip_prefix(&p.directory)
                        .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?,
                ),
                admin: admin.clone(),
                gitfile,
                config,
            });
        }
        // An uninitialized empty checkout may also move. Nonempty unregistered
        // directories must not be mistaken for a known, clean checkout.
        if !links.iter().any(|l| l.directory == p.directory)
            && fs::read_dir(&p.directory)
                .map_err(|e| Error::io("inspect uninitialized submodule", e))?
                .next()
                .is_some()
        {
            return fail(SubmoduleFailure::DestinationOccupied);
        }
        Ok(MovePlan {
            destination: dest,
            links,
        })
    }

    pub(super) fn apply_submodule_move(
        &self,
        p: &SubmoduleActionPreview,
        c: &CancellationToken,
    ) -> Result<SubmoduleActionReport> {
        if p.root != self.cwd() || !p.parent.starts_with(self.cwd()) {
            return fail(SubmoduleFailure::StalePreview);
        }
        let plan = p
            .relocation
            .as_ref()
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
        let _root = self.acquire_write(c)?;
        let parent = self.client().open(&p.parent, c)?;
        let _parent = if parent.info().common_dir != self.info().common_dir {
            Some(parent.acquire_write(c)?)
        } else {
            None
        };
        let children = plan
            .links
            .iter()
            .map(|l| self.client().open(&l.directory, c))
            .collect::<Result<Vec<_>>>()?;
        let _children = children
            .iter()
            .map(|r| r.acquire_write(c))
            .collect::<Result<Vec<_>>>()?;
        let mut pins = local_files::pin_directories(parent.cwd(), p.directory.parent().unwrap())?;
        pins.extend(local_files::pin_directories(
            parent.cwd(),
            plan.destination.parent().unwrap(),
        )?);
        for link in &plan.links {
            pins.extend(local_files::pin_directories(
                &parent.info().common_dir,
                &link.admin,
            )?);
        }
        let revalidate = || {
            if self.submodule_action_context(
                &p.request,
                p.target.as_ref(),
                p.source_root.as_deref(),
                c,
            )? != *p
            {
                return fail(SubmoduleFailure::StalePreview);
            }
            Ok(())
        };
        self.runner
            .step(p.label(), "revalidate_submodule_move", revalidate)?;
        let mut recovery = None;
        let mut completed = Vec::new();
        let mut publication = None;
        let result = self.runner.observe("write_result", p.label(), || {
            let workspace = allocate(&parent.info().git_dir)?;
            recovery = Some(workspace.clone());
            let _pin = local_files::pin_directories(&parent.info().git_dir, &workspace)?;
            self.runner.step(p.label(), "backup_submodule_move", || {
                files::backup(p, &workspace)?;
                let mut description = format!("Move recovery\nOriginal checkout: {}\nNew checkout: {}\n\nThe original directory was moved without replacing a destination. Do not restore metadata over newer edits. Inspect completed steps and both locations first. No automatic rollback runs.\n", p.directory.display(), plan.destination.display());
                for (i, link) in plan.links.iter().enumerate() {
                    link.gitfile.check()?; link.config.check()?;
                    files::write_new(&workspace.join(format!("link-{i}.before")), link.gitfile.bytes.as_ref().unwrap())?;
                    files::write_new(&workspace.join(format!("config-{i}.before")), link.config.bytes.as_ref().unwrap())?;
                    description.push_str(&format!("\nlink-{i}.before: {} -> {}\nconfig-{i}.before: {}\n", link.gitfile.path.display(), link.destination.join(".git").display(), link.config.path.display()));
                }
                files::write_new(&workspace.join("MOVE-RECOVERY.md"), description.as_bytes())
            })?;
            let mut metadata = parent.prepare_submodule_metadata(p, &workspace, c)?;
            let mut configs = files::Publication::new();
            let mut gitfiles = files::Publication::new();
            for (i, link) in plan.links.iter().enumerate() {
                c.check()?;
                let config = workspace.join(format!("config-{i}.after"));
                files::write_new(&config, link.config.bytes.as_ref().unwrap())?;
                let dest = git_path(&link.destination).to_string_lossy().replace('\\', "/");
                parent.edit_submodule_config(&config, &["--replace-all", "core.worktree", &dest], false, c)?;
                configs.add(&link.config, config, "move_child_config_published")?;
                let gitfile = workspace.join(format!("link-{i}.after"));
                files::write_new(&gitfile, format!("gitdir: {}\n", git_path(&link.admin).to_string_lossy().replace('\\', "/")).as_bytes())?;
                let mut moved = link.gitfile.clone(); moved.path = link.destination.join(".git");
                gitfiles.add(&moved, gitfile, "move_gitfile_published")?;
            }
            self.runner.step(p.label(), "revalidate_submodule_move", revalidate)?;
            metadata.lock(c)?; configs.lock(c)?;
            for state in [&p.index, &p.modules, &p.config].into_iter().chain(p.shared_index.iter()) { state.check()?; }
            for link in &plan.links { link.gitfile.check()?; link.config.check()?; }
            for child in &children { if !child.submodule_status(c)?.is_empty() { return fail(SubmoduleFailure::Dirty); } }
            if parent.head(c)? != p.head { return fail(SubmoduleFailure::StalePreview); }
            absent(&plan.destination)?;
            c.check()?;
            // The user can stop preparation. After this non-replacing rename,
            // finish the bounded prepared filesystem changes with a fresh token.
            // A disk/lock/external-edit failure is explicit and keeps recovery.
            let finish = CancellationToken::new();
            self.runner.step(p.label(), "move_submodule_checkout", || local_files::move_path(&p.directory, &plan.destination, false))?;
            completed.push("checkout_moved");
            let publish = (|| {
                let _dest_pin = local_files::pin_directories(parent.cwd(), &plan.destination)?;
                gitfiles.lock(&finish)?;
                gitfiles.publish(&parent, p.label(), &finish, &mut completed)?;
                configs.publish(&parent, p.label(), &finish, &mut completed)?;
                metadata.publish(&parent, p.label(), &finish, &mut completed)
            })();
            gitfiles.unlock(); configs.unlock(); metadata.unlock();
            publication = Some((metadata, configs, gitfiles));
            publish
        });
        let fresh = CancellationToken::new();
        let observed = self.runner.step(p.label(), "observe_submodule_action", || {
            self.submodules(&fresh)
        });
        let verification = self.runner.step(p.label(), "verify_submodule_move", || {
            if result.is_err() {
                return fail(SubmoduleFailure::VerificationFailed);
            }
            let (metadata, configs, gitfiles) = publication
                .as_ref()
                .ok_or(Error::Submodule(SubmoduleFailure::VerificationFailed))?;
            metadata.verify()?;
            configs.verify()?;
            gitfiles.verify()?;
            if parent.head(&fresh)? != p.head || p.directory.exists() {
                return fail(SubmoduleFailure::VerificationFailed);
            }
            let all = observed
                .as_ref()
                .map_err(|_| Error::Submodule(SubmoduleFailure::VerificationFailed))?;
            for old in p
                .entries
                .iter()
                .filter(|e| e.directory.starts_with(&p.directory))
            {
                let dest = plan
                    .destination
                    .join(old.directory.strip_prefix(&p.directory).unwrap());
                if !all.iter().any(|e| {
                    e.directory == dest
                        && e.target == old.target
                        && e.head == old.head
                        && e.initialized == old.initialized
                        && e.blocked.is_none()
                        && !e.dirty
                }) {
                    return fail(SubmoduleFailure::VerificationFailed);
                }
            }
            Ok(())
        });
        if let Some(log) = &self.runner.log {
            log.submodule_action_outcome(
                p.label(),
                &completed,
                result.is_ok(),
                verification.is_ok(),
            );
        }
        Ok(SubmoduleActionReport {
            result,
            verification,
            observed,
            recovery_directory: recovery,
            completed_steps: completed,
        })
    }
}
