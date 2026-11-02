use crate::{
    local_files,
    runner::{args, git_path},
    *,
};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct InitReport {
    pub destination: PathBuf,
    /// Retained on failure; never recursively deleted.
    pub staging_directory: PathBuf,
    pub preparation: Result<()>,
    pub publication: Option<Result<()>>,
    pub repository: Option<Result<Repository>>,
    pub cleanup: Option<Result<()>>,
}

/// The top-level entries [`check_init_destination`] counts at most.
pub const INIT_ENTRY_LIMIT: usize = 1000;

/// What New repository would do with a folder, decided as `init_repository` decides it
/// but without writing anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitDestination {
    /// The folder does not exist yet; its parent does.
    New,
    /// An existing folder whose files stay as they are, with its top-level entry count
    /// up to [`INIT_ENTRY_LIMIT`].
    Existing { entries: usize },
    /// A repository's working tree, or a folder inside one: the folder holding `.git`.
    Repository(PathBuf),
    /// A folder above it does not exist.
    MissingParent,
}

/// Check a New repository folder without changing anything. Other refusals (a drive
/// root, a link, a reserved name, Git metadata) are errors worded as for creating it.
pub fn check_init_destination(path: &Path) -> Result<InitDestination> {
    let destination = match destination_path(path) {
        Err(Error::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(InitDestination::MissingParent);
        }
        other => other?,
    };
    if let Some(Enclosing::Worktree(root)) = enclosing(&destination)? {
        return Ok(InitDestination::Repository(root));
    }
    reject_repository(&destination)?;
    match fs::read_dir(&destination) {
        Ok(entries) => Ok(InitDestination::Existing {
            entries: entries.take(INIT_ENTRY_LIMIT).count(),
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(InitDestination::New),
        Err(e) => Err(Error::io("inspect repository destination", e)),
    }
}

impl GitClient {
    /// Initialize a new folder or preserve all files in an existing project.
    /// Metadata is prepared in a sibling workspace and published without replacement.
    /// Existing repositories, nested repositories and redirected folders are refused.
    pub fn init_repository(
        &self,
        destination: impl AsRef<Path>,
        branch: &str,
        cancel: &CancellationToken,
    ) -> Result<InitReport> {
        let (destination, existed) = self.runner.step("init", "validate_init", || {
            cancel.check()?;
            if !cfg!(windows) {
                return Err(Error::Unsupported(
                    "Repository creation currently requires Windows".into(),
                ));
            }
            let destination = destination_path(destination.as_ref())?;
            let existed = fs::symlink_metadata(&destination).is_ok();
            reject_repository(&destination)?;
            if branch.is_empty()
                || branch.len() > 1024
                || branch.starts_with('-')
                || branch.chars().any(char::is_control)
            {
                return Err(Error::InvalidInput(
                    "Enter a valid initial branch name".into(),
                ));
            }
            let checked = self.runner.run(
                destination.parent().unwrap(),
                &args(&["check-ref-format", &format!("refs/heads/{branch}")]),
                vec![],
                cancel,
            )?;
            if checked.status.code() == Some(1) {
                return Err(Error::InvalidInput(
                    "Initial branch name is invalid; use a name such as main or feature/start"
                        .into(),
                ));
            }
            checked.success("validate initial branch")?;
            Ok((destination, existed))
        })?;
        // Keep existing directory names stable while preparing/publishing. These
        // handles deny Windows directory deletion/rename but allow ordinary file I/O.
        let _guards = guard_directories(if existed {
            &destination
        } else {
            destination.parent().unwrap()
        })?;
        let workspace = self.runner.step("init", "allocate_workspace", || {
            cancel.check()?;
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            for _ in 0..16 {
                let path = destination.parent().unwrap().join(format!(
                    ".rocket-init-{}-{time:x}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(path),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(Error::io("prepare repository workspace", e)),
                }
            }
            Err(Error::InvalidInput(
                "Could not allocate a unique repository workspace".into(),
            ))
        })?;
        let checkout = workspace.join("repository");
        let preparation = self.runner.step("init", "prepare_repository", || {
            let mut arguments = args(&["-c", "core.bare=false", "-c", "core.worktree=..", "init", "--template=", "--object-format=sha1", "--ref-format=files"]);
            arguments.push(format!("--initial-branch={branch}").into());
            arguments.extend(["--".into(), git_path(&checkout)]);
            self.runner.write(&workspace, &arguments, vec![], cancel)?.success("init")?;
            // Pin a relative worktree so inherited core.worktree cannot redirect
            // this repository after publication. No user identity is copied.
            let mut config = args(&["config", "--file"]);
            config.push(git_path(&checkout.join(".git/config")));
            config.extend(args(&["--no-includes", "core.worktree", ".."]));
            self.runner.write(&workspace, &config, vec![], cancel)?.success("initialize worktree setting")?;
            let repo = self.open(&checkout, cancel)?;
            let root = checkout.canonicalize().map_err(|e| Error::io("verify new repository", e))?;
            if repo.info().worktree.as_ref() != Some(&root) || repo.info().git_dir != root.join(".git") || repo.info().common_dir != root.join(".git")
                || !matches!(repo.head(cancel)?, Head::Unborn { branch: name } if name == branch.as_bytes()) {
                return Err(Error::InvalidInput("New repository verification failed; metadata was not published".into()));
            }
            Ok(())
        });
        let mut report = InitReport {
            destination,
            staging_directory: workspace,
            preparation,
            publication: None,
            repository: None,
            cleanup: None,
        };
        if report.preparation.is_ok() {
            report.publication = Some(self.runner.step("init", "publish_repository", || {
                cancel.check()?;
                reject_repository(&report.destination)?;
                local_files::directory(&checkout)?;
                local_files::directory(&checkout.join(".git"))?;
                if existed {
                    local_files::directory(&report.destination)?;
                    local_files::move_path(
                        &checkout.join(".git"),
                        &report.destination.join(".git"),
                        false,
                    )
                } else {
                    local_files::move_path(&checkout, &report.destination, false)
                }
            }));
        }
        if matches!(report.publication, Some(Ok(()))) {
            // Publication has happened. Reconcile with an independent token even
            // if cancellation arrived at that boundary; never report rollback.
            report.repository = Some(self.runner.step("init", "open_repository", || {
                self.open(&report.destination, &CancellationToken::new())
            }));
            report.cleanup = Some(self.runner.step("init", "cleanup_workspace", || {
                if existed {
                    fs::remove_dir(&checkout)
                        .map_err(|e| Error::io("remove empty init checkout", e))?;
                }
                fs::remove_dir(&report.staging_directory)
                    .map_err(|e| Error::io("remove empty init workspace", e))
            }));
        }
        Ok(report)
    }
}

pub(crate) fn destination_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::InvalidInput(
            "Choose an absolute folder path with an existing parent".into(),
        ));
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err(Error::InvalidInput(
            "Choose a folder on a local drive".into(),
        ));
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(Error::InvalidInput(
                    "Use a direct folder path without . or .. components".into(),
                ));
            }
            Component::Normal(name) => {
                let text = name.to_string_lossy();
                let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
                if text.ends_with(['.', ' '])
                    || text
                        .chars()
                        .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                    || matches!(
                        stem.as_str(),
                        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                    )
                    || ["COM", "LPT"].iter().any(|p| {
                        stem.strip_prefix(p).is_some_and(|s| {
                            matches!(
                                s,
                                "1" | "2"
                                    | "3"
                                    | "4"
                                    | "5"
                                    | "6"
                                    | "7"
                                    | "8"
                                    | "9"
                                    | "¹"
                                    | "²"
                                    | "³"
                            )
                        })
                    })
                {
                    return Err(Error::InvalidInput(
                        "Folder path contains an invalid or reserved Windows name".into(),
                    ));
                }
            }
            _ => {}
        }
    }
    let leaf = path
        .file_name()
        .ok_or_else(|| Error::InvalidInput("Choose a project folder, not a drive root".into()))?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidInput("Choose a folder with an existing parent".into()))?;
    for ancestor in parent.ancestors() {
        local_files::directory(ancestor)?;
    }
    let destination = parent
        .canonicalize()
        .map_err(|e| Error::io("resolve repository parent", e))?
        .join(leaf);
    match fs::symlink_metadata(&destination) {
        Ok(_) => local_files::directory(&destination)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect repository destination", e)),
    }
    Ok(destination)
}

/// The nearest Git metadata at or above a folder: a `.git` in a folder (a working
/// tree), or a folder that is itself Git metadata.
enum Enclosing {
    Worktree(PathBuf),
    Metadata,
}

fn enclosing(path: &Path) -> Result<Option<Enclosing>> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => return Ok(Some(Enclosing::Worktree(ancestor.to_path_buf()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("inspect existing Git metadata", e)),
        }
        if ancestor.join("HEAD").exists()
            && ancestor.join("objects").is_dir()
            && ancestor.join("refs").is_dir()
        {
            return Ok(Some(Enclosing::Metadata));
        }
    }
    Ok(None)
}

pub(crate) fn reject_repository(path: &Path) -> Result<()> {
    match enclosing(path)? {
        Some(Enclosing::Worktree(_)) => Err(Error::InvalidInput(
            "This folder is already a repository or is inside one. Use Open Repository instead"
                .into(),
        )),
        Some(Enclosing::Metadata) => Err(Error::InvalidInput("This folder is inside an existing Git metadata directory; choose a different location".into())),
        None => Ok(()),
    }
}

#[cfg(windows)]
pub(crate) fn guard_directories(path: &Path) -> Result<Vec<fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    path.ancestors()
        .map(|p| {
            let file = fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02000000 | 0x00200000)
                .open(p)
                .map_err(|e| Error::io("hold repository folder during creation", e))?;
            let meta = file
                .metadata()
                .map_err(|e| Error::io("inspect held repository folder", e))?;
            if !meta.is_dir() || local_files::redirected(&meta) {
                return Err(Error::InvalidInput(
                    "Repository folder changed or was redirected; choose it again".into(),
                ));
            }
            Ok(file)
        })
        .collect()
}
#[cfg(not(windows))]
pub(crate) fn guard_directories(_: &Path) -> Result<Vec<fs::File>> {
    Ok(vec![])
}
