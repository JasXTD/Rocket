use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileState {
    pub path: PathBuf,
    pub bytes: Option<Vec<u8>>,
}
impl FileState {
    pub fn read(path: PathBuf) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = match options.open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { path, bytes: None });
            }
            Err(e) => return Err(Error::io("read submodule metadata", e)),
        };
        let meta = file
            .metadata()
            .map_err(|e| Error::io("inspect submodule metadata", e))?;
        if !meta.is_file() || local_files::redirected(&meta) {
            return fail(SubmoduleFailure::InvalidLayout);
        }
        let mut bytes = Vec::new();
        file.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("read submodule metadata", e))?;
        if bytes.len() > 64 * 1024 * 1024 {
            return fail(SubmoduleFailure::Limit);
        }
        Ok(Self {
            path,
            bytes: Some(bytes),
        })
    }
    pub fn check(&self) -> Result<()> {
        if Self::read(self.path.clone())? != *self {
            return fail(SubmoduleFailure::StalePreview);
        }
        Ok(())
    }
}
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("create submodule recovery file", e))?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| Error::io("save submodule recovery file", e))
}
pub(super) fn backup(p: &SubmoduleActionPreview, workspace: &Path) -> Result<()> {
    let mut description = format!(
        "Rocket submodule recovery\nAction: {}\nParent: {}\nCheckout location: {}\n\nFiles below are the pre-operation bytes. An absent file has no backup.\nDo not restore over newer edits. Inspect Git status and compare first.\nFor removal, checkout/ retains the original directory and local commits.\nA gitfile checkout must be moved back to its original path before use.\nNo automatic rollback has run.\n",
        p.label(),
        p.parent.display(),
        p.directory.display()
    );
    for (name, state) in [
        ("index.before", &p.index),
        ("gitmodules.before", &p.modules),
        ("parent-config.before", &p.config),
    ]
    .into_iter()
    .chain(p.child_config.iter().map(|s| ("child-config.before", s)))
    .chain(p.shared_index.iter().map(|s| ("shared-index.before", s)))
    {
        state.check()?;
        if let Some(bytes) = &state.bytes {
            write_new(&workspace.join(name), bytes)?;
        }
        description.push_str(&format!(
            "{name} -> {} ({})\n",
            state.path.display(),
            if state.bytes.is_some() {
                "present"
            } else {
                "absent"
            }
        ));
    }
    write_new(&workspace.join("RECOVERY.md"), description.as_bytes())
}
struct Lock {
    path: PathBuf,
    file: Option<File>,
}
impl Lock {
    fn acquire(path: &Path) -> Result<Self> {
        let mut name = path.as_os_str().to_owned();
        name.push(".lock");
        let path = PathBuf::from(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3);
        }
        let file = options.open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Submodule(SubmoduleFailure::Busy)
            } else {
                Error::io("lock submodule metadata", e)
            }
        })?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        // Windows keeps this exact directory entry pinned until the handle closes.
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}
struct Change {
    before: FileState,
    prepared: PathBuf,
    bytes: Vec<u8>,
    step: &'static str,
}
pub(super) struct Publication {
    changes: Vec<Change>,
    locks: Vec<Lock>,
}
impl Publication {
    pub(super) fn new() -> Self {
        Self {
            changes: Vec::new(),
            locks: Vec::new(),
        }
    }
    pub(super) fn add(&mut self, before: &FileState, prepared: PathBuf, step: &'static str) -> Result<()> {
        let bytes = FileState::read(prepared.clone())?
            .bytes
            .ok_or(Error::Submodule(SubmoduleFailure::InvalidLayout))?;
        self.changes.push(Change {
            before: before.clone(),
            prepared,
            bytes,
            step,
        });
        Ok(())
    }
    pub fn lock(&mut self, c: &CancellationToken) -> Result<()> {
        let mut paths = self
            .changes
            .iter()
            .map(|s| &s.before.path)
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        for path in paths {
            c.check()?;
            self.locks.push(Lock::acquire(path)?);
        }
        for change in &self.changes {
            change.before.check()?;
        }
        Ok(())
    }
    pub fn unlock(&mut self) {
        self.locks.clear();
    }
    pub fn publish(
        &mut self,
        repo: &Repository,
        label: &'static str,
        c: &CancellationToken,
        completed: &mut Vec<&'static str>,
    ) -> Result<()> {
        for change in &self.changes {
            c.check()?;
            change.before.check()?;
            repo.runner.step(label, change.step, || {
                local_files::move_path(
                    &change.prepared,
                    &change.before.path,
                    change.before.bytes.is_some(),
                )
            })?;
            completed.push(change.step);
        }
        Ok(())
    }
    pub fn verify(&self) -> Result<()> {
        for change in &self.changes {
            if FileState::read(change.before.path.clone())?.bytes.as_ref() != Some(&change.bytes) {
                return fail(SubmoduleFailure::VerificationFailed);
            }
        }
        Ok(())
    }
}

impl Repository {
    pub(super) fn edit_submodule_config(
        &self,
        file: &Path,
        arguments: &[&str],
        optional_section: bool,
        c: &CancellationToken,
    ) -> Result<()> {
        let mut a = args(&["config", "--no-includes", "--file"]);
        a.push(git_path(file));
        a.extend(args(arguments));
        let o = self.runner.write(self.cwd(), &a, vec![], c)?;
        if optional_section
            && o.status.code() == Some(128)
            && o.stderr.starts_with(b"fatal: no such section:")
        {
            return Ok(());
        }
        if optional_section && o.status.code() == Some(5) {
            return Ok(());
        }
        o.success("submodule-config")?;
        Ok(())
    }
    pub(super) fn prepare_submodule_metadata(
        &self,
        p: &SubmoduleActionPreview,
        workspace: &Path,
        c: &CancellationToken,
    ) -> Result<Publication> {
        let mut out = Publication::new();
        let config = workspace.join("parent-config.after");
        write_new(&config, p.config.bytes.as_deref().unwrap_or_default())?;
        let section = format!("submodule.{}", p.name);
        match &p.request {
            SubmoduleAction::Add { .. } => {
                self.edit_submodule_config(
                    &config,
                    &["--replace-all", &format!("{section}.url"), &p.source],
                    false,
                    c,
                )?;
                self.edit_submodule_config(
                    &config,
                    &["--replace-all", &format!("{section}.active"), "true"],
                    false,
                    c,
                )?;
            }
            SubmoduleAction::Remove(_) | SubmoduleAction::Deinitialize(_) => {
                self.edit_submodule_config(&config, &["--remove-section", &section], true, c)?
            }
            SubmoduleAction::Sync(_) | SubmoduleAction::SetUrl { .. } => self.edit_submodule_config(
                &config,
                &["--replace-all", &format!("{section}.url"), &p.source],
                false,
                c,
            )?,
            SubmoduleAction::Move { .. } => {}
        }
        if matches!(p.request, SubmoduleAction::Deinitialize(_)) {
            out.add(&p.config, config, "registration_removed")?;
            return Ok(out);
        }
        if matches!(p.request, SubmoduleAction::Sync(_) | SubmoduleAction::SetUrl { .. }) {
            if let (Some(state), Some(remote)) = (&p.child_config, &p.child_remote) {
                let file = workspace.join("child-config.after");
                write_new(&file, state.bytes.as_deref().unwrap_or_default())?;
                self.edit_submodule_config(
                    &file,
                    &["--replace-all", &format!("remote.{remote}.url"), &p.source],
                    false,
                    c,
                )?;
                out.add(state, file, "child_url_synced")?;
            }
            if matches!(p.request, SubmoduleAction::Sync(_)) {
                out.add(&p.config, config, "parent_url_synced")?;
                return Ok(out);
            }
        }
        let modules = workspace.join("gitmodules.after");
        write_new(&modules, p.modules.bytes.as_deref().unwrap_or_default())?;
        match &p.request {
            SubmoduleAction::Add { source, .. } => {
                self.edit_submodule_config(
                    &modules,
                    &["--replace-all", &format!("{section}.path"), &p.name],
                    false,
                    c,
                )?;
                self.edit_submodule_config(
                    &modules,
                    &["--replace-all", &format!("{section}.url"), source],
                    false,
                    c,
                )?;
            }
            SubmoduleAction::Remove(_) => {
                self.edit_submodule_config(&modules, &["--remove-section", &section], false, c)?
            }
            SubmoduleAction::SetUrl { source, .. } => {
                self.edit_submodule_config(
                    &modules, &["--replace-all", &format!("{section}.url"), source], false, c,
                )?;
            }
            SubmoduleAction::Move { destination, .. } => {
                let dest = std::str::from_utf8(destination.as_bytes()).map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?;
                self.edit_submodule_config(&modules, &["--replace-all", &format!("{section}.path"), dest], false, c)?;
            }
            _ => unreachable!(),
        }
        let bytes = FileState::read(modules.clone())?.bytes.unwrap();
        let oid = self
            .runner
            .write(
                self.cwd(),
                &args(&["hash-object", "-w", "--stdin", "--path=.gitmodules"]),
                bytes,
                c,
            )?
            .success("submodule-object")?;
        let oid = ObjectId::parse(
            std::str::from_utf8(&oid)
                .map_err(|_| Error::Submodule(SubmoduleFailure::InvalidLayout))?
                .trim(),
        )?;
        let index = workspace.join("index.after");
        if let Some(bytes) = &p.index.bytes {
            write_new(&index, bytes)?;
        } else {
            self.runner
                .private_index(
                    self.cwd(),
                    &index,
                    &args(&["read-tree", "--empty"]),
                    vec![],
                    c,
                )?
                .success("submodule-index")?;
        }
        self.runner
            .private_index(
                self.cwd(),
                &index,
                &args(&["update-index", "--no-split-index"]),
                vec![],
                c,
            )?
            .success("submodule-index")?;
        let mut input = format!("{} {}\t.gitmodules\0", p.modules_mode, oid).into_bytes();
        match p.request {
            SubmoduleAction::Add { .. } => {
                input.extend(format!("160000 {}\t", p.target.as_ref().unwrap()).as_bytes())
            }
            SubmoduleAction::Remove(_) => {
                input.extend(format!("0 {}\t", "0".repeat(oid.as_str().len())).as_bytes())
            }
            SubmoduleAction::SetUrl { .. } => {}
            SubmoduleAction::Move { ref destination, .. } => {
                input.extend(format!("0 {}\t", "0".repeat(oid.as_str().len())).as_bytes());
                input.extend(p.relative.as_bytes());
                input.push(0);
                input.extend(format!("160000 {}\t", p.target.as_ref().unwrap()).as_bytes());
                input.extend(destination.as_bytes());
                input.push(0);
            }
            _ => unreachable!(),
        }
        if !matches!(p.request, SubmoduleAction::SetUrl { .. } | SubmoduleAction::Move { .. }) {
            input.extend(p.relative.as_bytes());
            input.push(0);
        }
        self.runner
            .private_index(
                self.cwd(),
                &index,
                &args(&["update-index", "-z", "--index-info"]),
                input,
                c,
            )?
            .success("submodule-index")?;
        // All work is prepared before any live metadata is changed. Index last:
        // a failure cannot falsely report a fully staged operation.
        out.add(&p.modules, modules, "gitmodules_published")?;
        if !matches!(p.request, SubmoduleAction::Move { .. }) {
            out.add(&p.config, config, "parent_config_published")?;
        }
        out.add(&p.index, index, "index_published")?;
        Ok(out)
    }
}
