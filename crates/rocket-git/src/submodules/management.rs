//! Metadata changes are prepared in a private index and published under Git
//! locks. A cancelled/partial publication is retained for inspection, not rolled
//! back over a possibly newer external edit.
mod files;
mod relocate;
use super::*;
use files::FileState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmoduleAction {
    Add {
        parent: PathBuf,
        path: RepoPath,
        source: String,
    },
    Remove(RepoPath),
    Deinitialize(RepoPath),
    Sync(RepoPath),
    SetUrl { path: RepoPath, source: String },
    /// Destination is relative to the selected module's immediate parent repo.
    Move { path: RepoPath, destination: RepoPath },
}
impl SubmoduleAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Add { .. } => "submodule-add",
            Self::Remove(_) => "submodule-remove",
            Self::Deinitialize(_) => "submodule-deinitialize",
            Self::Sync(_) => "submodule-sync",
            Self::SetUrl { .. } => "submodule-set-url",
            Self::Move { .. } => "submodule-move",
        }
    }
}

// All fields are sealed: consumers may display a preview but cannot retarget it.
#[derive(Clone, PartialEq, Eq)]
pub struct SubmoduleActionPreview {
    request: SubmoduleAction,
    source_root: Option<PathBuf>,
    root: PathBuf,
    parent: PathBuf,
    directory: PathBuf,
    relative: RepoPath,
    name: String,
    source: String,
    target: Option<ObjectId>,
    entries: Vec<SubmoduleEntry>,
    head: Head,
    index: FileState,
    shared_index: Option<FileState>,
    modules: FileState,
    config: FileState,
    child_config: Option<FileState>,
    child_remote: Option<String>,
    previous_urls: Vec<String>,
    modules_mode: String,
    relocation: Option<relocate::MovePlan>,
}
impl std::fmt::Debug for SubmoduleActionPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Config snapshots can contain credentials; even an accidental Debug of
        // the UI Action must not expose the sealed metadata payload.
        f.debug_struct("SubmoduleActionPreview")
            .field("operation", &self.label())
            .finish_non_exhaustive()
    }
}
impl SubmoduleActionPreview {
    pub fn action(&self) -> &SubmoduleAction {
        &self.request
    }
    pub fn label(&self) -> &'static str {
        self.request.label()
    }
    pub fn parent(&self) -> &Path {
        &self.parent
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn path(&self) -> &RepoPath {
        &self.relative
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn move_destination(&self) -> Option<&Path> {
        self.relocation.as_ref().map(|m| m.destination.as_path())
    }
    pub fn local_source(&self) -> bool {
        local_source(&self.source)
    }
    pub fn target(&self) -> Option<&ObjectId> {
        self.target.as_ref()
    }
    pub fn previous_urls(&self) -> &[String] {
        &self.previous_urls
    }
    pub fn descendants(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                e.directory != self.directory
                    && e.directory.starts_with(&self.directory)
                    && e.initialized
            })
            .count()
    }
}
pub struct SubmoduleActionReport {
    pub result: Result<()>,
    pub verification: Result<()>,
    pub observed: Result<Vec<SubmoduleEntry>>,
    pub recovery_directory: Option<PathBuf>,
    pub completed_steps: Vec<&'static str>,
}

impl Repository {
    pub fn preview_submodule_action(
        &self,
        action: &SubmoduleAction,
        cancel: &CancellationToken,
    ) -> Result<SubmoduleActionPreview> {
        self.runner
            .step(action.label(), "preview_submodule_action", || {
                self.submodule_action_context(action, None, None, cancel)
            })
    }

    /// Automation variant: resolve rewrites before any source access and require
    /// the effective source to remain a local repository inside the test root.
    pub fn preview_submodule_action_local(
        &self,
        action: &SubmoduleAction,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<SubmoduleActionPreview> {
        let root = root
            .canonicalize()
            .map_err(|e| Error::io("resolve submodule fixture root", e))?;
        self.runner
            .step(action.label(), "preview_submodule_action", || {
                self.submodule_action_context(action, None, Some(&root), cancel)
            })
    }

    fn submodule_action_context(
        &self,
        action: &SubmoduleAction,
        pinned: Option<&ObjectId>,
        source_root: Option<&Path>,
        c: &CancellationToken,
    ) -> Result<SubmoduleActionPreview> {
        self.require_worktree(c)?;
        let entries = self.submodules(c)?;
        let selected = match action {
            SubmoduleAction::Add { .. } => None,
            SubmoduleAction::Remove(path) | SubmoduleAction::Deinitialize(path) | SubmoduleAction::Sync(path) | SubmoduleAction::SetUrl { path, .. } | SubmoduleAction::Move { path, .. } => Some(
                entries
                    .iter()
                    .find(|e| &e.path == path)
                    .ok_or(Error::Submodule(SubmoduleFailure::StalePreview))?,
            ),
        };
        if matches!(action, SubmoduleAction::Deinitialize(_)) && selected.is_none_or(|e|!e.initialized) {
            return fail(SubmoduleFailure::RegistrationMissing);
        }
        let parent_path = match action {
            SubmoduleAction::Add { parent, .. } => {
                let parent = parent
                    .canonicalize()
                    .map_err(|e| Error::io("resolve submodule parent", e))?;
                if parent != self.cwd()
                    && !entries
                        .iter()
                        .any(|e| e.directory == parent && e.initialized)
                {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
                parent
            }
            _ => selected.unwrap().parent.clone(),
        };
        let parent = self.client().open(&parent_path, c)?;
        if parent.cwd() != parent_path {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        if let Some(f) = parent.submodule_blocker(c)? {
            return fail(f);
        }
        let index = FileState::read(parent.info().git_dir.join("index"))?;
        let shared = parent
            .runner
            .run(
                parent.cwd(),
                &args(&["rev-parse", "--shared-index-path"]),
                vec![],
                c,
            )?
            .success("submodule-index")?;
        let shared = std::str::from_utf8(&shared)
            .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?
            .trim_end_matches(['\r', '\n']);
        let shared_index = if shared.is_empty() {
            None
        } else {
            let shared = parent
                .cwd()
                .join(shared)
                .canonicalize()
                .map_err(|e| Error::io("resolve shared submodule index", e))?;
            if shared.parent() != Some(parent.info().git_dir.as_path())
                || !shared
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("sharedindex."))
            {
                return fail(SubmoduleFailure::InvalidLayout);
            }
            Some(FileState::read(shared)?)
        };
        let modules = FileState::read(parent.cwd().join(".gitmodules"))?;
        let config = FileState::read(parent.info().common_dir.join("config"))?;
        if config.bytes.is_none() {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let mappings = if modules.bytes.as_ref().is_none_or(Vec::is_empty) {
            BTreeMap::new()
        } else {
            parent.modules_config(c)?
        };
        let (relative, name, directory) = match action {
            SubmoduleAction::Add { path, .. } => {
                let directory = parent.submodule_path(path.as_bytes())?;
                // Resolve existing ancestry as well as spelling: a Windows short
                // name must not hide a path into Git's administration directory.
                let existing = existing_parent(
                    directory
                        .parent()
                        .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?,
                )?;
                let resolved = existing
                    .canonicalize()
                    .map_err(|e| Error::io("resolve submodule destination ancestry", e))?;
                if !resolved.starts_with(parent.cwd())
                    || resolved.starts_with(&parent.info().git_dir)
                    || resolved.starts_with(&parent.info().common_dir)
                {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
                if fs::symlink_metadata(&directory).is_ok() {
                    return fail(SubmoduleFailure::DestinationOccupied);
                }
                let name = std::str::from_utf8(path.as_bytes())
                    .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?
                    .to_owned();
                if mappings
                    .iter()
                    .any(|(p, (n, _))| p == path.as_bytes() || n == &name)
                {
                    return fail(SubmoduleFailure::DestinationOccupied);
                }
                // Do not add through an unregistered embedded repository.
                let mut ancestor = directory.parent();
                while let Some(p) = ancestor.filter(|p| *p != parent.cwd()) {
                    if p.join(".git").exists() {
                        return fail(SubmoduleFailure::InvalidLayout);
                    }
                    ancestor = p.parent();
                }
                let tracked = parent
                    .runner
                    .run(
                        parent.cwd(),
                        &args(&["ls-files", "-z", "--", &name]),
                        vec![],
                        c,
                    )?
                    .success("submodule-index")?;
                if !tracked.is_empty() {
                    return fail(SubmoduleFailure::DestinationOccupied);
                }
                (path.clone(), name, directory)
            }
            _ => {
                let e = selected.unwrap();
                (e.relative_path.clone(), e.name.clone(), e.directory.clone())
            }
        };
        let section = format!("submodule.{name}.");
        parent.submodule_local_section(&section, &config.path, c)?;
        if matches!(action, SubmoduleAction::Add { .. })
            && !parent
                .submodule_config_section(&section, Some(&config.path), c)?
                .is_empty()
        {
            return fail(SubmoduleFailure::RetainedMetadata);
        }
        if matches!(action, SubmoduleAction::Add { .. })
            && modules.bytes.is_some()
            && !parent
                .submodule_config_section(&section, Some(&modules.path), c)?
                .is_empty()
        {
            return fail(SubmoduleFailure::RetainedMetadata);
        }
        if matches!(action, SubmoduleAction::Add { .. })
            && parent
                .info()
                .common_dir
                .join("modules")
                .join(&name)
                .exists()
        {
            return fail(SubmoduleFailure::RetainedMetadata);
        }
        let mut modules_mode = "100644".to_owned();
        if !matches!(action, SubmoduleAction::Sync(_)) {
            parent.submodule_metadata_clean(
                &relative,
                matches!(action, SubmoduleAction::Remove(_) | SubmoduleAction::Deinitialize(_) | SubmoduleAction::Move { .. }),
                c,
            )?;
            let index_entries = parent.submodule_index(c)?;
            let recorded = index_entries
                .split(|b| *b == 0)
                .find(|r| r.ends_with(b"\t.gitmodules"));
            if let Some(line) = recorded {
                let mode = line.split(|b| *b == b' ').next().unwrap_or_default();
                if !matches!(mode, b"100644" | b"100755") {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
                modules_mode = String::from_utf8_lossy(mode).into_owned();
            } else if modules.bytes.is_some() {
                return fail(SubmoduleFailure::MetadataEdits);
            }
            // A metadata edit must never invoke a configured filter program.
            let attrs = parent
                .runner
                .run(
                    parent.cwd(),
                    &args(&[
                        "check-attr",
                        "-z",
                        "filter",
                        "working-tree-encoding",
                        "--",
                        ".gitmodules",
                    ]),
                    vec![],
                    c,
                )?
                .success("submodule-attributes")?;
            if attrs
                .split(|b| *b == 0)
                .collect::<Vec<_>>()
                .chunks(3)
                .any(|r| r.len() == 3 && !matches!(r[2], b"unspecified" | b"unset"))
            {
                return fail(SubmoduleFailure::InvalidLayout);
            }
        }
        if matches!(action, SubmoduleAction::Remove(_) | SubmoduleAction::Deinitialize(_) | SubmoduleAction::Move { .. }) {
            for e in entries
                .iter()
                .filter(|e| e.directory.starts_with(&directory))
            {
                if let Some(f) = e.blocked {
                    return fail(f);
                }
                if e.dirty {
                    return fail(SubmoduleFailure::Dirty);
                }
            }
        }
        let mut source = String::new();
        let mut target = selected.map(|e| e.target.clone());
        let mut child_config = None;
        let mut child_remote = None;
        let mut previous_urls = Vec::new();
        match action {
            SubmoduleAction::Add {
                source: requested, ..
            } => {
                source = parent.resolve_submodule_source(requested, c)?;
                if let Some(root) = source_root
                    && (!local_source(&source)
                        || !Path::new(&source)
                            .canonicalize()
                            .is_ok_and(|p| p.starts_with(root)))
                {
                    return fail(SubmoduleFailure::UnsupportedSource);
                }
                target = if let Some(id) = pinned {
                    Some(id.clone())
                } else {
                    parent.check_submodule_transport(&source, c)?;
                    let bytes = parent
                        .runner
                        .clone_command(
                            parent.cwd(),
                            &args(&["ls-remote", "--exit-code", "--", &source, "HEAD"]),
                            c,
                            None,
                            local_source(&source),
                        )?
                        .success("submodule-fetch")?;
                    let rows = bytes
                        .split(|b| *b == b'\n')
                        .filter(|r| !r.is_empty())
                        .collect::<Vec<_>>();
                    if rows.len() != 1 {
                        return fail(SubmoduleFailure::InvalidLayout);
                    }
                    let (id, name) = rows[0]
                        .split_once_byte(b'\t')
                        .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
                    if name != b"HEAD" {
                        return fail(SubmoduleFailure::InvalidLayout);
                    }
                    Some(ObjectId::parse(std::str::from_utf8(id).map_err(|_| {
                        Error::Submodule(SubmoduleFailure::InvalidLayout)
                    })?)?)
                };
                let width = if parent.info().object_format == ObjectFormat::Sha1 {
                    40
                } else {
                    64
                };
                if target.as_ref().unwrap().as_str().len() != width {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
            }
            SubmoduleAction::Sync(_) | SubmoduleAction::SetUrl { .. } => {
                let (_, url) = mappings
                    .get(relative.as_bytes())
                    .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
                let requested = match action {
                    SubmoduleAction::SetUrl { source, .. } => source,
                    _ => url,
                };
                source = parent.resolve_submodule_source(requested, c)?;
                if let Some(root) = source_root
                    && (!local_source(&source)
                        || !Path::new(&source).canonicalize().is_ok_and(|p| p.starts_with(root)))
                {
                    return fail(SubmoduleFailure::UnsupportedSource);
                }
                parent.check_submodule_transport(&source, c)?;
                let old = parent
                    .submodule_config_value(&format!("submodule.{name}.url"), c)?
                    .ok_or(Error::Submodule(SubmoduleFailure::RegistrationMissing))?;
                previous_urls.push(old);
                let e = selected.unwrap();
                if let Some(child) = parent.submodule_child(e, c)? {
                    if let Some(f) = child.submodule_blocker(c)? {
                        return fail(f);
                    }
                    let remote = match child.head(c)? {
                        Head::Branch { name, .. } => child
                            .submodule_config_value(
                                &format!("branch.{}.remote", String::from_utf8_lossy(&name)),
                                c,
                            )?
                            .unwrap_or_else(|| "origin".into()),
                        _ => "origin".into(),
                    };
                    if remote == "." || remote.chars().any(char::is_control) {
                        return fail(SubmoduleFailure::ConfigScope);
                    }
                    let state = FileState::read(child.info().common_dir.join("config"))?;
                    child.submodule_local_section(&format!("remote.{remote}."), &state.path, c)?;
                    previous_urls.push(
                        child
                            .submodule_config_value(&format!("remote.{remote}.url"), c)?
                            .ok_or(Error::Submodule(SubmoduleFailure::ConfigScope))?,
                    );
                    child.check_submodule_transport(&source, c)?;
                    child_config = Some(state);
                    child_remote = Some(remote);
                }
            }
            SubmoduleAction::Remove(_) | SubmoduleAction::Deinitialize(_) | SubmoduleAction::Move { .. } => {}
        }
        let head = parent.head(c)?;
        for state in [&index, &modules, &config]
            .into_iter()
            .chain(child_config.iter())
            .chain(shared_index.iter())
        {
            state.check()?;
        }
        let mut preview = SubmoduleActionPreview {
            request: action.clone(),
            source_root: source_root.map(Path::to_owned),
            root: self.cwd().to_owned(),
            parent: parent_path,
            directory,
            relative,
            name,
            source,
            target,
            entries,
            head,
            index,
            shared_index,
            modules,
            config,
            child_config,
            child_remote,
            previous_urls,
            modules_mode,
            relocation: None,
        };
        if let SubmoduleAction::Move { destination, .. } = action {
            preview.relocation = Some(parent.plan_submodule_move(&preview, destination, c)?);
        }
        Ok(preview)
    }

    fn submodule_metadata_clean(
        &self,
        path: &RepoPath,
        removing: bool,
        c: &CancellationToken,
    ) -> Result<()> {
        for cached in [false, true] {
            let mut a = args(&[
                "diff",
                // Read-only helpers disable autoRefreshIndex. --quiet can then
                // report a stat-only difference after identical bytes are restored.
                // Generate the bounded metadata diff to compare actual content.
                "--exit-code",
                "--patch",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=none",
            ]);
            if cached {
                a.push("--cached".into());
            }
            a.extend(args(&["--", ".gitmodules"]));
            if cached && removing {
                a.push(path.argument()?);
            }
            let out = self.runner.run(self.cwd(), &a, vec![], c)?;
            if out.status.code() == Some(1) && !out.stdout.is_empty() {
                return fail(SubmoduleFailure::MetadataEdits);
            }
            // With autoRefreshIndex disabled, Git can keep exit status 1 for a
            // stat-only change even after its content comparison emits no patch.
            if out.status.code() == Some(1) { continue; }
            out.success("submodule-metadata")?;
        }
        Ok(())
    }

    fn submodule_config_section(
        &self,
        prefix: &str,
        file: Option<&Path>,
        c: &CancellationToken,
    ) -> Result<Vec<Vec<u8>>> {
        let mut a = args(&["config", "--null", "--list"]);
        if let Some(file) = file {
            a.extend(args(&["--no-includes", "--file"]));
            a.push(git_path(file));
        }
        let bytes = self
            .runner
            .run(self.cwd(), &a, vec![], c)?
            .success("submodule-config")?;
        Ok(bytes
            .split(|b| *b == 0)
            .filter(|r| r.starts_with(prefix.as_bytes()))
            .map(<[u8]>::to_vec)
            .collect())
    }
    fn submodule_local_section(
        &self,
        prefix: &str,
        file: &Path,
        c: &CancellationToken,
    ) -> Result<()> {
        if self.submodule_config_section(prefix, None, c)?
            != self.submodule_config_section(prefix, Some(file), c)?
        {
            return fail(SubmoduleFailure::ConfigScope);
        }
        Ok(())
    }

    pub fn apply_submodule_action(
        &self,
        p: &SubmoduleActionPreview,
        c: &CancellationToken,
    ) -> Result<SubmoduleActionReport> {
        if matches!(p.request, SubmoduleAction::Move { .. }) {
            return self.apply_submodule_move(p, c);
        }
        if p.root != self.cwd()
            || !p.parent.starts_with(self.cwd())
            || p.directory == p.parent
            || !p.directory.starts_with(&p.parent)
        {
            return fail(SubmoduleFailure::StalePreview);
        }
        let _root = self.acquire_write(c)?;
        let parent = self.client().open(&p.parent, c)?;
        let _parent = if parent.info().common_dir != self.info().common_dir {
            Some(parent.acquire_write(c)?)
        } else {
            None
        };
        let selected = p.entries.iter().find(|e| e.directory == p.directory);
        let child = selected
            .map(|e| parent.submodule_child(e, c))
            .transpose()?
            .flatten();
        let _child = child.as_ref().map(|r| r.acquire_write(c)).transpose()?;
        // Sync writes the child config in place. Hold both its working-folder
        // ancestry and contained admin ancestry against junction/rename swaps.
        // Removal deliberately cannot pin the directory it must move.
        let mut sync_paths = Vec::new();
        if matches!(p.request, SubmoduleAction::Sync(_) | SubmoduleAction::SetUrl { .. })
            && let Some(child) = &child
        {
            sync_paths.extend(local_files::pin_directories(parent.cwd(), child.cwd())?);
            let admin_root = if child.info().git_dir.starts_with(child.cwd()) {
                child.cwd()
            } else {
                &parent.info().common_dir
            };
            sync_paths.extend(local_files::pin_directories(
                admin_root,
                &child.info().git_dir,
            )?);
        }
        let revalidate = || {
            self.runner
                .step(p.label(), "revalidate_submodule_action", || {
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
                })
        };
        revalidate()?;
        let mut recovery = None;
        let mut completed = Vec::new();
        let mut publication = None;
        let result = self.runner.observe("write_result", p.label(), || {
            let workspace = allocate(&parent.info().git_dir)?;
            recovery = Some(workspace.clone());
            let _pin = local_files::pin_directories(&parent.info().git_dir, &workspace)?;
            self.runner
                .step(p.label(), "backup_submodule_metadata", || {
                    files::backup(p, &workspace)
                })?;
            let checkout = workspace.join("checkout");
            if matches!(p.request, SubmoduleAction::Add { .. }) {
                self.runner.step(p.label(), "prepare_submodule_add", || {
                    parent.prepare_submodule_add(p, &workspace, &checkout, c)
                })?;
            }
            let prepared = self
                .runner
                .step(p.label(), "prepare_submodule_metadata", || {
                    parent.prepare_submodule_metadata(p, &workspace, c)
                })?;
            revalidate()?;
            let mut prepared = prepared;
            self.runner
                .step(p.label(), "lock_submodule_metadata", || prepared.lock(c))?;
            // Our own locks now exist. Recheck bytes and HEAD without the normal
            // lock-blocker test; the complete preview was checked just above.
            for state in [&p.index, &p.modules, &p.config]
                .into_iter()
                .chain(p.child_config.iter())
                .chain(p.shared_index.iter())
            {
                state.check()?;
            }
            if parent.head(c)? != p.head {
                return fail(SubmoduleFailure::StalePreview);
            }
            let dest_parent = p
                .directory
                .parent()
                .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
            create_parents(parent.cwd(), dest_parent)?;
            let _paths = local_files::pin_directories(parent.cwd(), dest_parent)?;
            c.check()?;
            match &p.request {
                SubmoduleAction::Add { .. } => {
                    self.runner
                        .step(p.label(), "publish_submodule_checkout", || {
                            local_files::move_path(&checkout, &p.directory, false)
                        })?;
                    completed.push("checkout_published");
                }
                SubmoduleAction::Remove(_) | SubmoduleAction::Deinitialize(_) if p.directory.exists() => {
                    if let Some(child) = &child {
                        if !child.submodule_status(c)?.is_empty() {
                            return fail(SubmoduleFailure::Dirty);
                        }
                    } else if fs::read_dir(&p.directory)
                        .map_err(|e| Error::io("check submodule folder", e))?
                        .next()
                        .is_some()
                    {
                        return fail(SubmoduleFailure::DestinationOccupied);
                    }
                    // A non-replacing, same-volume move preserves every file and
                    // local commit, including an external edit racing this check.
                    self.runner
                        .step(p.label(), "retain_submodule_checkout", || {
                            local_files::move_path(&p.directory, &checkout, false)
                        })?;
                    completed.push("checkout_retained");
                    if matches!(p.request, SubmoduleAction::Deinitialize(_)) {
                        c.check()?;
                        fs::create_dir(&p.directory).map_err(|e|Error::io("create empty deinitialized submodule folder",e))?;
                        completed.push("empty_checkout_created");
                    }
                }
                _ => {}
            }
            let write_result = self
                .runner
                .step(p.label(), "publish_submodule_metadata", || {
                    prepared.publish(&parent, p.label(), c, &mut completed)
                });
            publication = Some(prepared);
            write_result
        });
        // Locks must be released before the fresh-token observation, even after
        // a cancelled process or a partially published operation.
        if let Some(prepared) = &mut publication {
            prepared.unlock();
        }
        let fresh = CancellationToken::new();
        let mut observer = self.clone();
        observer.runner.limits.timeout = CommandLimits::default().timeout;
        let observed = self.runner.step(p.label(), "observe_submodule_action", || {
            observer.submodules(&fresh)
        });
        let verification = self.runner.step(p.label(), "verify_submodule_action", || {
            if result.is_err() {
                return fail(SubmoduleFailure::VerificationFailed);
            }
            publication
                .as_ref()
                .ok_or(Error::Submodule(SubmoduleFailure::VerificationFailed))?
                .verify()?;
            if parent.head(&fresh)? != p.head {
                return fail(SubmoduleFailure::VerificationFailed);
            }
            let all = observed
                .as_ref()
                .map_err(|_| Error::Submodule(SubmoduleFailure::VerificationFailed))?;
            match &p.request {
                SubmoduleAction::Add { .. }
                    if !all.iter().any(|e| {
                        e.directory == p.directory
                            && e.head == p.target
                            && Some(&e.target) == p.target.as_ref()
                    }) =>
                {
                    return fail(SubmoduleFailure::VerificationFailed);
                }
                SubmoduleAction::Remove(_) if all.iter().any(|e| e.directory == p.directory) => {
                    return fail(SubmoduleFailure::VerificationFailed);
                }
                SubmoduleAction::Deinitialize(_) if !all.iter().any(|e| e.directory == p.directory && !e.initialized) => {
                    return fail(SubmoduleFailure::VerificationFailed);
                }
                _ => {}
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

    fn prepare_submodule_add(
        &self,
        p: &SubmoduleActionPreview,
        workspace: &Path,
        checkout: &Path,
        c: &CancellationToken,
    ) -> Result<()> {
        let template = workspace.join("empty-template");
        fs::create_dir(&template).map_err(|e| Error::io("create submodule template", e))?;
        let mut a = args(&[
            "-c",
            "submodule.recurse=false",
            "-c",
            "core.longpaths=true",
            "clone",
            "--config=core.longpaths=true",
            "--no-checkout",
            "--no-local",
            "--no-hardlinks",
            "--no-recurse-submodules",
        ]);
        let mut option = std::ffi::OsString::from("--template=");
        option.push(git_path(&template));
        a.push(option);
        a.extend(args(&["--", &p.source]));
        a.push(git_path(checkout));
        self.check_submodule_transport(&p.source, c)?;
        self.runner
            .clone_command(self.cwd(), &a, c, None, p.local_source())?
            .success("submodule-init")?;
        let prepared = self.client().open(checkout, c)?;
        let id = p
            .target
            .as_ref()
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
        prepared
            .runner
            .write(
                prepared.cwd(),
                &args(&[
                    "-c",
                    "submodule.recurse=false",
                    "checkout",
                    "--detach",
                    "--no-recurse-submodules",
                    "--no-overwrite-ignore",
                    id.as_str(),
                    "--",
                ]),
                vec![],
                c,
            )?
            .success("submodule-checkout")?;
        if prepared.head(c)? != (Head::Detached { target: id.clone() }) {
            return fail(SubmoduleFailure::VerificationFailed);
        }
        Ok(())
    }
}
