//! Reviewed submodule checkouts and metadata actions. Initialization/update never
//! stage the parent; management add/remove explicitly stage reviewed metadata.
//! Nested checkouts are discovered and reviewed independently, without force.
mod management;
mod batch;
pub use batch::{SubmoduleBatchPreview, SubmoduleBatchReport};
use crate::{
    local_files,
    model::bytes_to_os,
    runner::{args, git_path},
    *,
};
pub use management::{SubmoduleAction, SubmoduleActionPreview, SubmoduleActionReport};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const LIMIT: usize = 128;
const DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmoduleFailure {
    InvalidLayout,
    Dirty,
    Busy,
    HiddenIndex,
    NestedLayoutChange,
    StalePreview,
    DestinationOccupied,
    RetainedMetadata,
    UnsupportedSource,
    Limit,
    CommandFailed,
    VerificationFailed,
    MetadataEdits,
    ConfigScope,
    RegistrationMissing,
}
impl SubmoduleFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidLayout => "submodule_invalid_layout",
            Self::Dirty => "submodule_dirty",
            Self::Busy => "submodule_busy",
            Self::HiddenIndex => "submodule_hidden_index",
            Self::NestedLayoutChange => "submodule_nested_layout_change",
            Self::StalePreview => "submodule_stale_preview",
            Self::DestinationOccupied => "submodule_destination_occupied",
            Self::RetainedMetadata => "submodule_retained_metadata",
            Self::UnsupportedSource => "submodule_unsupported_source",
            Self::Limit => "submodule_limit",
            Self::CommandFailed => "submodule_command_failed",
            Self::VerificationFailed => "submodule_verification_failed",
            Self::MetadataEdits => "submodule_metadata_edits",
            Self::ConfigScope => "submodule_config_scope",
            Self::RegistrationMissing => "submodule_registration_missing",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::MetadataEdits => {
                "Commit or unstage/restore existing changes to .gitmodules and the selected gitlink before adding or removing a submodule. Rocket stages only the reviewed metadata; unrelated files stay unchanged."
            }
            Self::ConfigScope => {
                "The selected URL settings are missing, ambiguous or supplied by an include, global or worktree config. Define unambiguous repository-local URLs with Git before retrying; Rocket does not edit other config files."
            }
            Self::RegistrationMissing => {
                "This submodule has no local URL registration to sync. Initialize it first, or register it with Git submodule init, then refresh. Sync does not initialize a checkout."
            }
            Self::InvalidLayout => {
                "Submodule metadata or paths are invalid, conflicted, redirected, or outside this repository. Inspect .gitmodules and the index with Git; no path is followed for writing."
            }
            Self::Dirty => {
                "The selected submodule or an initialized descendant has staged, modified, untracked or ignored files. Commit, stash or move those files, then refresh and review again. Rocket never forces an update."
            }
            Self::Busy => {
                "A merge, rebase or another Git operation is active. Finish it and inspect existing locks before retrying; Rocket does not remove locks."
            }
            Self::HiddenIndex => {
                "The submodule uses skip-worktree or assume-unchanged entries. Clear those flags and review its files before updating."
            }
            Self::NestedLayoutChange => {
                "The target removes or moves an initialized nested submodule. Inspect and relocate or deinitialize that checkout explicitly with Git before updating its parent; Rocket will not remove it."
            }
            Self::StalePreview => {
                "The recorded commit, source, parent index or checkout changed after review. Refresh and review again; do not reuse this preview."
            }
            Self::DestinationOccupied => {
                "The destination or uninitialized submodule folder already exists or contains files. Choose an unused destination or move existing files to a safe location first; Rocket will not replace or delete them."
            }
            Self::RetainedMetadata => {
                "A previous submodule initialization left Git metadata without a usable checkout. Inspect it with Git before retrying; Rocket retains it and does not force reuse or cleanup."
            }
            Self::UnsupportedSource => {
                "Use a local repository path, HTTPS, SSH or an SSH-style user@host:path source without embedded passwords. Unsupported transports and ambiguous sources are refused."
            }
            Self::Limit => {
                "Submodule discovery reached 128 entries or 16 nested levels, or metadata exceeded the 64 MiB management limit. Open an initialized child to inspect a smaller subtree, or manage oversized metadata with Git."
            }
            Self::CommandFailed => {
                "Git could not complete the submodule operation. Check source access and authentication, then refresh. Downloads or initialization may remain; no automatic retry or cleanup was performed."
            }
            Self::VerificationFailed => {
                "The observed checkout does not match the reviewed result. Refresh and inspect it before retrying; cancellation is not rollback."
            }
        }
    }
}
fn fail<T>(f: SubmoduleFailure) -> Result<T> {
    Err(Error::Submodule(f))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmoduleEntry {
    pub path: RepoPath,
    pub parent: PathBuf,
    pub directory: PathBuf,
    pub relative_path: RepoPath,
    pub name: String,
    pub depth: usize,
    pub target: ObjectId,
    pub head: Option<ObjectId>,
    pub branch: Option<Vec<u8>>,
    pub url: String,
    pub dirty: bool,
    pub initialized: bool,
    pub blocked: Option<SubmoduleFailure>,
    git_dir: Option<PathBuf>,
}
impl SubmoduleEntry {
    pub fn state(&self) -> &'static str {
        if self.blocked.is_some() {
            "Needs attention"
        } else if self.dirty {
            "Local changes"
        } else if !self.initialized {
            "Not initialized"
        } else if self.head.as_ref() != Some(&self.target) {
            "Update available"
        } else {
            "Up to date"
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmodulePreview {
    pub entry: SubmoduleEntry,
    pub source: String,
    pub local_source: bool,
    root: PathBuf,
    parent_head: Head,
    parent_index: Vec<u8>,
    modules: Vec<u8>,
    descendants: Vec<SubmoduleEntry>,
}
impl SubmodulePreview {
    pub fn label(&self) -> &'static str {
        if self.entry.initialized {
            "submodule-update"
        } else {
            "submodule-init"
        }
    }
}
pub struct SubmoduleWriteReport {
    pub result: Result<()>,
    pub observed: Result<Vec<SubmoduleEntry>>,
    pub verification: Result<()>,
    pub recovery_reference: Option<String>,
    pub retained_directory: Option<PathBuf>,
}

impl Repository {
    /// Bounded read of gitlinks in each parent's index, not the remote branch tip.
    pub fn submodules(&self, cancel: &CancellationToken) -> Result<Vec<SubmoduleEntry>> {
        self.require_worktree(cancel)?;
        self.runner.step("submodules", "discover_submodules", || {
            let mut entries = Vec::new();
            let mut visited = BTreeSet::new();
            self.scan_submodules(self.cwd(), 0, &mut entries, &mut visited, cancel)?;
            Ok(entries)
        })
    }
    fn client(&self) -> GitClient {
        GitClient::from_runner(self.runner.clone())
    }
    fn scan_submodules(
        &self,
        root: &Path,
        depth: usize,
        out: &mut Vec<SubmoduleEntry>,
        visited: &mut BTreeSet<PathBuf>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        cancel.check()?;
        if depth > DEPTH || !visited.insert(self.info().git_dir.clone()) {
            return fail(SubmoduleFailure::Limit);
        }
        let links = self.submodule_links(cancel)?;
        if links.is_empty() {
            return Ok(());
        }
        let modules = self.modules_config(cancel)?;
        for (path, target) in links {
            cancel.check()?;
            if out.len() >= LIMIT {
                return fail(SubmoduleFailure::Limit);
            }
            let (name, default_url) = modules
                .get(&path)
                .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
            validate_path(name.as_bytes())?;
            let directory = self.submodule_path(&path)?;
            let relative = directory
                .strip_prefix(root)
                .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?;
            let display = relative.to_string_lossy().replace('\\', "/");
            let configured =
                self.submodule_config_value(&format!("submodule.{name}.url"), cancel)?;
            let mut e = SubmoduleEntry {
                path: RepoPath::new(display.as_bytes().to_vec())?,
                relative_path: RepoPath::new(path)?,
                parent: self.cwd().to_owned(),
                directory: directory.clone(),
                name: name.clone(),
                depth,
                target,
                head: None,
                branch: None,
                url: configured.unwrap_or_else(|| default_url.clone()),
                dirty: false,
                initialized: false,
                blocked: None,
                git_dir: None,
            };
            let child = self.submodule_child(&e, cancel)?;
            if let Some(child) = &child {
                e.initialized = true;
                e.git_dir = Some(child.info().git_dir.clone());
                match child.head(cancel)? {
                    Head::Branch { name, target } => {
                        e.head = Some(target);
                        e.branch = Some(name)
                    }
                    Head::Detached { target } => e.head = Some(target),
                    Head::Unborn { branch } => e.branch = Some(branch),
                }
                e.dirty = !child.submodule_status(cancel)?.is_empty();
                e.blocked = child.submodule_blocker(cancel)?;
            } else if directory.exists()
                && fs::read_dir(&directory)
                    .map_err(|e| Error::io("inspect submodule folder", e))?
                    .next()
                    .is_some()
            {
                e.blocked = Some(SubmoduleFailure::DestinationOccupied)
            }
            if child.is_none() && self.info().common_dir.join("modules").join(name).exists() {
                e.blocked = Some(SubmoduleFailure::RetainedMetadata)
            }
            out.push(e);
            if let Some(child) = child {
                child.scan_submodules(root, depth + 1, out, visited, cancel)?;
            }
        }
        Ok(())
    }
    pub fn preview_submodule(
        &self,
        entry: &SubmoduleEntry,
        cancel: &CancellationToken,
    ) -> Result<SubmodulePreview> {
        self.runner
            .step("submodule-preview", "validate_submodule_preview", || {
                let all = self.submodules(cancel)?;
                let current = all
                    .iter()
                    .find(|e| e.path == entry.path)
                    .ok_or(Error::Submodule(SubmoduleFailure::StalePreview))?;
                if current != entry {
                    return fail(SubmoduleFailure::StalePreview);
                }
                if let Some(f) = current.blocked {
                    return fail(f);
                }
                let descendants = all
                    .iter()
                    .filter(|e| {
                        e.directory.starts_with(&entry.directory) && e.directory != entry.directory
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if current.dirty || descendants.iter().any(|e| e.dirty) {
                    return fail(SubmoduleFailure::Dirty);
                }
                if let Some(f) = descendants.iter().find_map(|e| e.blocked) {
                    return fail(f);
                }
                let parent = self.client().open(&entry.parent, cancel)?;
                if !parent.operation_state(cancel)?.is_idle() {
                    return fail(SubmoduleFailure::Busy);
                }
                let mut source = parent.resolve_submodule_source(&current.url, cancel)?;
                if let Some(child) = parent.submodule_child(current, cancel)? {
                    // Fetch runs in the child, whose conditional includes and URL
                    // rewrites may differ from the parent's configuration.
                    source = child.resolve_submodule_source(&source, cancel)?;
                }
                let local_source = local_source(&source);
                Ok(SubmodulePreview {
                    entry: current.clone(),
                    source,
                    local_source,
                    root: self.cwd().to_owned(),
                    parent_head: parent.head(cancel)?,
                    parent_index: parent.submodule_index(cancel)?,
                    modules: parent.modules_bytes()?,
                    descendants,
                })
            })
    }
    pub fn open_submodule(
        &self,
        entry: &SubmoduleEntry,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        self.runner
            .step("submodule-open", "validate_submodule_open", || {
                let all = self.submodules(cancel)?;
                let e = all
                    .iter()
                    .find(|e| e.path == entry.path)
                    .ok_or(Error::Submodule(SubmoduleFailure::StalePreview))?;
                if e != entry || !e.initialized {
                    return fail(SubmoduleFailure::StalePreview);
                }
                Ok(e.directory.clone())
            })
    }
    pub fn apply_submodule(
        &self,
        p: &SubmodulePreview,
        cancel: &CancellationToken,
    ) -> Result<SubmoduleWriteReport> {
        let _root_permit = self.acquire_write(cancel)?;
        self.apply_submodule_locked(p, cancel)
    }
    fn apply_submodule_locked(&self, p: &SubmodulePreview, cancel: &CancellationToken) -> Result<SubmoduleWriteReport> {
        // Public preview fields may have been altered by a backend caller. Check
        // containment before acquiring ancestor/child gates, including self-locks.
        if p.root != self.cwd()
            || !p.entry.parent.starts_with(self.cwd())
            || p.entry.directory == p.entry.parent
            || !p.entry.directory.starts_with(&p.entry.parent)
        {
            return fail(SubmoduleFailure::StalePreview);
        }
        let parent = self.client().open(&p.entry.parent, cancel)?;
        let _parent_permit = if parent.info().common_dir != self.info().common_dir {
            Some(parent.acquire_write(cancel)?)
        } else {
            None
        };
        let child = parent.submodule_child(&p.entry, cancel)?;
        let _child_permit = child
            .as_ref()
            .map(|c| c.acquire_write(cancel))
            .transpose()?;
        let revalidate = || {
            self.runner.step(p.label(), "revalidate_submodule", || {
                if p.root != self.cwd() || self.preview_submodule(&p.entry, cancel)? != *p {
                    return fail(SubmoduleFailure::StalePreview);
                }
                Ok(())
            })
        };
        revalidate()?;
        let parent_dir = p
            .entry
            .directory
            .parent()
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
        // Pin existing ancestors against rename/junction replacement on Windows.
        let existing = existing_parent(parent_dir)?;
        let _paths = local_files::pin_directories(parent.cwd(), &existing)?;
        let mut recovery = None;
        let mut retained = None;
        let result = self.runner.observe("write_result", p.label(), || {
            if let Some(child) = &child {
                self.runner.step(p.label(), "fetch_recorded_commit", || {
                    if child
                        .resolve_commit(p.entry.target.as_str(), cancel)
                        .is_err()
                    {
                        cancel.check()?;
                        let a = args(&[
                            "-c",
                            "submodule.recurse=false",
                            "fetch",
                            "--no-tags",
                            "--no-recurse-submodules",
                            "--no-write-fetch-head",
                            "--no-auto-maintenance",
                            "--",
                            &p.source,
                            p.entry.target.as_str(),
                        ]);
                        // Restrict protocols after all user-configured URL rewrites.
                        child.check_submodule_transport(&p.source, cancel)?;
                        child
                            .runner
                            .clone_command(child.cwd(), &a, cancel, None, p.local_source)?
                            .success("submodule-fetch")?;
                    }
                    Ok(())
                })?;
                revalidate()?;
                self.runner.step(p.label(), "check_nested_layout", || {
                    let tree = child
                        .runner
                        .run(
                            child.cwd(),
                            &args(&["ls-tree", "-r", "-z", p.entry.target.as_str()]),
                            vec![],
                            cancel,
                        )?
                        .success("submodule-tree")?;
                    for entry in p
                        .descendants
                        .iter()
                        .filter(|e| e.initialized && e.parent == p.entry.directory)
                    {
                        if !tree.split(|b| *b == 0).any(|r| {
                            r.starts_with(b"160000 ")
                                && r.split_once_byte(b'\t')
                                    .is_some_and(|(_, path)| path == entry.relative_path.as_bytes())
                        }) {
                            return fail(SubmoduleFailure::NestedLayoutChange);
                        }
                    }
                    Ok(())
                })?;
                if let Some(old) = &p.entry.head {
                    let reference = format!("refs/rocket/submodule-checkouts/{old}");
                    self.runner.step(p.label(), "retain_submodule_tip", || {
                        let symbolic = child.runner.run(
                            child.cwd(),
                            &args(&["symbolic-ref", "--quiet", &reference]),
                            vec![],
                            cancel,
                        )?;
                        if symbolic.status.code() != Some(1) {
                            return fail(SubmoduleFailure::StalePreview);
                        }
                        let existing = child.resolve_commit(&reference, cancel).ok();
                        if existing.as_ref().is_some_and(|id| id != old) {
                            return fail(SubmoduleFailure::StalePreview);
                        }
                        let zero = "0".repeat(old.as_str().len());
                        child
                            .runner
                            .write(
                                child.cwd(),
                                &args(&[
                                    "update-ref",
                                    "--no-deref",
                                    &reference,
                                    old.as_str(),
                                    existing.as_ref().map_or(zero.as_str(), ObjectId::as_str),
                                ]),
                                vec![],
                                cancel,
                            )?
                            .success("submodule-retain")?;
                        Ok(())
                    })?;
                    recovery = Some(reference);
                }
                self.runner
                    .step(p.label(), "checkout_recorded_commit", || {
                        child
                            .runner
                            .write(
                                child.cwd(),
                                &args(&[
                                    "-c",
                                    "submodule.recurse=false",
                                    "checkout",
                                    "--detach",
                                    "--no-recurse-submodules",
                                    "--no-overwrite-ignore",
                                    p.entry.target.as_str(),
                                    "--",
                                ]),
                                vec![],
                                cancel,
                            )?
                            .success("submodule-checkout")?;
                        Ok(())
                    })?;
            } else {
                self.runner
                    .step(p.label(), "initialize_selected_submodule", || {
                        // Prepare away from the working folder. Git's initial submodule
                        // checkout may force a just-cloned checkout; never expose an
                        // existing folder to that behavior, including concurrent edits.
                        let workspace = allocate(self.info().git_dir.as_path())?;
                        retained = Some(workspace.clone());
                        let template = workspace.join("empty-template");
                        fs::create_dir(&template)
                            .map_err(|e| Error::io("create submodule template", e))?;
                        let checkout = workspace.join("checkout");
                        let mut a = args(&[
                            "-c",
                            "submodule.recurse=false",
                            "clone",
                            "--no-checkout",
                            "--no-local",
                            "--no-hardlinks",
                            "--no-recurse-submodules",
                        ]);
                        let mut option = std::ffi::OsString::from("--template=");
                        option.push(git_path(&template));
                        a.push(option);
                        a.extend(args(&["--", &p.source]));
                        a.push(git_path(&checkout));
                        parent.check_submodule_transport(&p.source, cancel)?;
                        parent
                            .runner
                            .clone_command(parent.cwd(), &a, cancel, None, p.local_source)?
                            .success("submodule-init")?;
                        let prepared = self.client().open(&checkout, cancel)?;
                        if prepared
                            .resolve_commit(p.entry.target.as_str(), cancel)
                            .is_err()
                        {
                            cancel.check()?;
                            prepared.check_submodule_transport(&p.source, cancel)?;
                            prepared
                                .runner
                                .clone_command(
                                    prepared.cwd(),
                                    &args(&[
                                        "fetch",
                                        "--no-tags",
                                        "--no-recurse-submodules",
                                        "--no-write-fetch-head",
                                        "--",
                                        &p.source,
                                        p.entry.target.as_str(),
                                    ]),
                                    cancel,
                                    None,
                                    p.local_source,
                                )?
                                .success("submodule-fetch")?;
                        }
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
                                    p.entry.target.as_str(),
                                    "--",
                                ]),
                                vec![],
                                cancel,
                            )?
                            .success("submodule-checkout")?;
                        revalidate()?;
                        create_parents(parent.cwd(), parent_dir)?;
                        let _publish_paths =
                            local_files::pin_directories(parent.cwd(), parent_dir)?;
                        self.runner.step(p.label(), "publish_submodule", || {
                            cancel.check()?;
                            // remove_dir removes only an empty directory. A new file
                            // aborts publication; rename never replaces a raced path.
                            if p.entry.directory.exists() {
                                local_files::directory(&p.entry.directory)?;
                                fs::remove_dir(&p.entry.directory).map_err(|_| {
                                    Error::Submodule(SubmoduleFailure::DestinationOccupied)
                                })?;
                            }
                            local_files::move_path(&checkout, &p.entry.directory, false)
                        })?;
                        // The prepared checkout keeps its ordinary embedded .git.
                        // Git supports this layout; no metadata move can strand it.
                        let mut a = args(&["submodule", "init", "--"]);
                        a.push(p.entry.relative_path.argument()?);
                        parent
                            .runner
                            .write(parent.cwd(), &a, vec![], cancel)?
                            .success("submodule-init")?;
                        Ok(())
                    })?;
            }
            Ok(())
        });
        let fresh = CancellationToken::new();
        let mut observer = self.clone();
        observer.runner.limits.timeout = CommandLimits::default().timeout;
        let observed = self.runner.step(p.label(), "observe_submodules", || {
            observer.submodules(&fresh)
        });
        let verification = self.runner.step(p.label(), "verify_submodule_result", || {
            let all = observed
                .as_ref()
                .map_err(|_| Error::Submodule(SubmoduleFailure::VerificationFailed))?;
            let e = all
                .iter()
                .find(|e| e.path == p.entry.path)
                .ok_or(Error::Submodule(SubmoduleFailure::VerificationFailed))?;
            if e.head.as_ref() != Some(&p.entry.target)
                || parent.submodule_index(&fresh)? != p.parent_index
                || parent.head(&fresh)? != p.parent_head
                || parent.modules_bytes()? != p.modules
            {
                return fail(SubmoduleFailure::VerificationFailed);
            }
            Ok(())
        });
        Ok(SubmoduleWriteReport {
            result,
            observed,
            verification,
            recovery_reference: recovery,
            retained_directory: retained,
        })
    }
    fn submodule_index(&self, c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(&["ls-files", "--stage", "-z"]), vec![], c)?
            .success("submodule-index")
    }
    fn submodule_links(&self, c: &CancellationToken) -> Result<Vec<(Vec<u8>, ObjectId)>> {
        let mut out = Vec::new();
        for line in self
            .submodule_index(c)?
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
        {
            let Some(tab) = line.iter().position(|b| *b == b'\t') else {
                return fail(SubmoduleFailure::InvalidLayout);
            };
            let fields = line[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
            if fields.len() != 3 {
                return fail(SubmoduleFailure::InvalidLayout);
            }
            if fields[0] == b"160000" {
                if fields[2] != b"0" {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
                let path = line[tab + 1..].to_vec();
                validate_path(&path)?;
                out.push((
                    path,
                    ObjectId::parse(
                        std::str::from_utf8(fields[1])
                            .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?,
                    )?,
                ));
            }
        }
        Ok(out)
    }
    fn modules_bytes(&self) -> Result<Vec<u8>> {
        let p = self.cwd().join(".gitmodules");
        if !p.exists() {
            return Ok(vec![]);
        }
        local_files::read_config(&p)
    }
    fn modules_config(&self, c: &CancellationToken) -> Result<BTreeMap<Vec<u8>, (String, String)>> {
        let before = self.modules_bytes()?;
        if before.is_empty() {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let b = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "config",
                    "--null",
                    "--no-includes",
                    "--file",
                    ".gitmodules",
                    "--list",
                ]),
                vec![],
                c,
            )?
            .success("submodule-config")?;
        if before != self.modules_bytes()? {
            return fail(SubmoduleFailure::StalePreview);
        }
        let mut names: BTreeMap<String, (Option<Vec<u8>>, Option<String>)> = BTreeMap::new();
        for r in b.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let (key, val) = r
                .split_once_byte(b'\n')
                .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
            let key = std::str::from_utf8(key)
                .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?;
            let Some(key) = key.strip_prefix("submodule.") else {
                continue;
            };
            let Some((name, field)) = key.rsplit_once('.') else {
                continue;
            };
            let entry = names.entry(name.to_owned()).or_default();
            match field {
                "path" => {
                    if entry.0.replace(val.to_vec()).is_some() {
                        return fail(SubmoduleFailure::InvalidLayout);
                    }
                }
                "url" => {
                    let url = std::str::from_utf8(val)
                        .map_err(|_| Error::Submodule(SubmoduleFailure::UnsupportedSource))?;
                    if entry.1.replace(url.to_owned()).is_some() {
                        return fail(SubmoduleFailure::InvalidLayout);
                    }
                }
                _ => {}
            }
        }
        let mut out = BTreeMap::new();
        for (name, (path, url)) in names {
            if let (Some(path), Some(url)) = (path, url) {
                validate_path(&path)?;
                if out.insert(path, (name, url)).is_some() {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
            }
        }
        Ok(out)
    }
    fn submodule_config_value(&self, key: &str, c: &CancellationToken) -> Result<Option<String>> {
        let o = self.runner.run(
            self.cwd(),
            &args(&["config", "--null", "--get-all", key]),
            vec![],
            c,
        )?;
        if o.status.code() == Some(1) && o.stdout.is_empty() {
            return Ok(None);
        }
        let b = o.success("submodule-config")?;
        let values = b
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>();
        if values.len() > 1 {
            // An include or duplicate config key needs configuration guidance,
            // even when discovered while building the read-only submodule list.
            return fail(SubmoduleFailure::ConfigScope);
        }
        if values.is_empty() {
            return fail(SubmoduleFailure::UnsupportedSource);
        }
        Ok(Some(String::from_utf8(values[0].to_vec()).map_err(
            |_| Error::Submodule(SubmoduleFailure::UnsupportedSource),
        )?))
    }
    fn submodule_path(&self, path: &[u8]) -> Result<PathBuf> {
        validate_path(path)?;
        let relative = bytes_to_os(path)?;
        let dest = self.cwd().join(relative);
        let mut p = self.cwd().to_owned();
        for part in Path::new(&bytes_to_os(path)?).components() {
            p.push(part);
            match fs::symlink_metadata(&p) {
                Ok(m) if local_files::redirected(&m) || !m.is_dir() => {
                    return fail(SubmoduleFailure::InvalidLayout);
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("inspect submodule path", e)),
            }
        }
        Ok(dest)
    }
    fn submodule_child(
        &self,
        e: &SubmoduleEntry,
        c: &CancellationToken,
    ) -> Result<Option<Repository>> {
        self.submodule_read_repository(&e.relative_path, c)
    }
    pub(crate) fn submodule_read_repository(&self, path: &RepoPath, c: &CancellationToken) -> Result<Option<Repository>> {
        let directory = self.submodule_path(path.as_bytes())?;
        let git = directory.join(".git");
        match fs::symlink_metadata(&git) {
            Err(x) if x.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(x) => return Err(Error::io("inspect submodule metadata", x)),
            Ok(m) if local_files::redirected(&m) => return fail(SubmoduleFailure::InvalidLayout),
            Ok(_) => {}
        }
        let child = self.client().open(&directory, c)?;
        if child.info().worktree.as_ref() != Some(&directory)
            || child.info().git_dir != child.info().common_dir
        {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let admin = &child.info().git_dir;
        let embedded = directory.join(".git");
        let modules = self.info().common_dir.join("modules");
        if admin != &embedded && !admin.starts_with(&modules) {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let root = if admin == &embedded {
            &directory
        } else {
            &self.info().common_dir
        };
        let _pin = local_files::pin_directories(root, admin)?;
        Ok(Some(child))
    }
    fn submodule_status(&self, c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(
                self.cwd(),
                &args(&[
                    "-c",
                    "core.untrackedCache=false",
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--untracked-files=all",
                    "--ignored=matching",
                    "--ignore-submodules=none",
                ]),
                vec![],
                c,
            )?
            .success("submodule-status")
    }
    fn submodule_blocker(&self, c: &CancellationToken) -> Result<Option<SubmoduleFailure>> {
        for lock in ["index.lock", "HEAD.lock", "config.lock"] {
            if self.info().git_dir.join(lock).exists() {
                return Ok(Some(SubmoduleFailure::Busy));
            }
        }
        if !self.operation_state(c)?.is_idle() {
            return Ok(Some(SubmoduleFailure::Busy));
        }
        let b = self
            .runner
            .run(self.cwd(), &args(&["ls-files", "-v", "-z"]), vec![], c)?
            .success("submodule-index-flags")?;
        Ok(b.split(|b| *b == 0)
            .any(|r| {
                r.first()
                    .is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')
            })
            .then_some(SubmoduleFailure::HiddenIndex))
    }
    fn resolve_submodule_source(&self, url: &str, c: &CancellationToken) -> Result<String> {
        let source = if url.starts_with("./") || url.starts_with("../") {
            let remote = match self.head(c)? {
                Head::Branch { name, .. } => self
                    .submodule_config_value(
                        &format!("branch.{}.remote", String::from_utf8_lossy(&name)),
                        c,
                    )?
                    .unwrap_or_else(|| "origin".into()),
                _ => "origin".into(),
            };
            let base = self
                .submodule_config_value(&format!("remote.{remote}.url"), c)?
                .unwrap_or_else(|| git_path(self.cwd()).to_string_lossy().into_owned());
            resolve_relative(&base, url)?
        } else {
            url.to_owned()
        };
        validate_source(&source)?;
        let b = self
            .runner
            .run(
                self.cwd(),
                &args(&["ls-remote", "--get-url", "--", &source]),
                vec![],
                c,
            )?
            .success("submodule-source")?;
        let effective = String::from_utf8(b)
            .map_err(|_| Error::Submodule(SubmoduleFailure::UnsupportedSource))?
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        validate_source(&effective)?;
        Ok(effective)
    }
    fn check_submodule_transport(&self, source: &str, cancel: &CancellationToken) -> Result<()> {
        self.runner
            .step("submodule-source", "recheck_effective_source", || {
                if self.resolve_submodule_source(source, cancel)? != source {
                    return fail(SubmoduleFailure::StalePreview);
                }
                Ok(())
            })
    }
}
fn existing_parent(path: &Path) -> Result<PathBuf> {
    let mut p = path;
    while !p.exists() {
        p = p
            .parent()
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
    }
    local_files::directory(p)?;
    Ok(p.to_owned())
}
fn allocate(parent: &Path) -> Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    for n in 0..64 {
        let p = parent.join(format!(
            "rocket-submodule-{}-{}-{n}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io("allocate submodule preparation", e)),
        }
    }
    fail(SubmoduleFailure::DestinationOccupied)
}
fn create_parents(root: &Path, parent: &Path) -> Result<()> {
    let rel = parent
        .strip_prefix(root)
        .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?;
    let mut p = root.to_owned();
    for component in rel.components() {
        p.push(component);
        match fs::create_dir(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io("create submodule parent", e)),
        }
        local_files::directory(&p)?;
    }
    Ok(())
}
fn validate_path(bytes: &[u8]) -> Result<()> {
    let s = std::str::from_utf8(bytes)
        .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?;
    if s.is_empty()
        || s.len() > 4096
        || s.chars().any(char::is_control)
        || s.contains([
            '\\', ':', '\0', '\n', '\r', '=', '"', '<', '>', '|', '?', '*',
        ])
        || s.split('/').any(|p| {
            p.is_empty()
                || matches!(p, "." | "..")
                || p.eq_ignore_ascii_case(".git")
                || reserved_component(p)
                || p.ends_with(['.', ' '])
        })
    {
        return fail(SubmoduleFailure::InvalidLayout);
    }
    Ok(())
}
fn reserved_component(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}
fn local_source(s: &str) -> bool {
    Path::new(s).is_absolute() && !s.contains("://")
}
fn validate_source(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 16384 || s.chars().any(char::is_control) || s.starts_with('-') {
        return fail(SubmoduleFailure::UnsupportedSource);
    }
    if local_source(s) {
        if s.starts_with("\\\\") || s.starts_with("//") {
            return fail(SubmoduleFailure::UnsupportedSource);
        }
        return Ok(());
    }
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("ssh://"))
    {
        let (authority, path) = rest
            .split_once('/')
            .ok_or(Error::Submodule(SubmoduleFailure::UnsupportedSource))?;
        if authority.is_empty()
            || path.is_empty()
            || s.chars().any(char::is_whitespace)
            || s.contains(['?', '#', '\\'])
            || (s.starts_with("https://") && authority.contains('@'))
            || authority
                .split_once('@')
                .is_some_and(|(u, _)| u.contains(':'))
        {
            return fail(SubmoduleFailure::UnsupportedSource);
        }
        return Ok(());
    }
    if let Some((host, path)) = s.split_once(':')
        && host.contains('@')
        && !host.contains(['/', '\\', ' '])
        && !path.is_empty()
        && !path.starts_with(':')
    {
        return Ok(());
    }
    fail(SubmoduleFailure::UnsupportedSource)
}
fn resolve_relative(base: &str, relative: &str) -> Result<String> {
    let mut base = base.replace('\\', "/").trim_end_matches('/').to_owned();
    let mut rest = relative;
    while let Some(tail) = rest.strip_prefix("../") {
        let pos = base
            .rfind('/')
            .ok_or(Error::Submodule(SubmoduleFailure::UnsupportedSource))?;
        base.truncate(pos);
        rest = tail;
    }
    while let Some(tail) = rest.strip_prefix("./") {
        rest = tail;
    }
    let result = format!("{base}/{rest}");
    validate_source(&result)?;
    Ok(result)
}
trait SplitByte {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])>;
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])> {
        let n = self.iter().position(|b| *b == byte)?;
        Some((&self[..n], &self[n + 1..]))
    }
}
