use crate::{
    CancellationToken, CloneFailure, ClonePhase, CloneProgress, CloneReport, CloneSource, Error,
    GitClient, Head, Result, WriteOutput,
    runner::{args, git_path, has_error_diagnostics},
};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::SyncSender,
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

impl GitClient {
    /// Clone into a fresh sibling workspace, verify, then publish without replacement.
    /// Existing destinations (including empty directories) are refused. After workspace
    /// creation, inspect every phase in the report; interruption is not rollback.
    ///
    /// This first implementation supports Windows publication. Progress is lossy and
    /// nonblocking; only the returned report establishes completion. No recursive
    /// cleanup, credential mutation, retry, or submodule clone is performed by Rocket.
    pub fn clone_repository(
        &self,
        source: &CloneSource,
        destination: impl AsRef<Path>,
        progress: Option<&SyncSender<CloneProgress>>,
        cancel: &CancellationToken,
    ) -> Result<CloneReport> {
        phase(progress, ClonePhase::Preparing);
        let local_only = matches!(source, CloneSource::Local(_));
        let (source, destination) = self.runner.step("clone", "validate_clone", || {
            cancel.check()?;
            if !cfg!(windows) {
                return Err(Error::Unsupported(
                    "clone publication currently requires Windows".into(),
                ));
            }
            let source = source_argument(source)?;
            let destination = destination_path(destination.as_ref())?;
            Ok((source, destination))
        })?;
        let workspace = self.runner.step("clone", "allocate_workspace", || {
            cancel.check()?;
            create_workspace(destination.parent().expect("validated parent"))
        })?;
        let checkout = workspace.join("repository");
        let template = workspace.join("empty-template");
        let command = self.runner.observe("clone_command", "clone", || {
            cancel.check()?;
            fs::create_dir(&template).map_err(|e| Error::io("prepare clone workspace", e))?;
            // Hooks are disabled for this invocation only. The empty template
            // prevents copying configured hooks into the newly created repository.
            let mut hooks_option = OsString::from("core.hooksPath=");
            hooks_option.push(git_path(&template));
            let mut arguments = vec![OsString::from("-c"), hooks_option];
            arguments.extend(args(&[
                "clone",
                "--progress",
                "--no-local",
                "--no-recurse-submodules",
                "--origin=origin",
                // Relative to .git, so inherited core.worktree cannot redirect the
                // checkout and moving the finished repository preserves its identity.
                "--config=core.worktree=..",
            ]));
            let mut template_option = OsString::from("--template=");
            template_option.push(git_path(&template));
            arguments.extend([template_option, "--".into(), source, git_path(&checkout)]);
            phase(progress, ClonePhase::Cloning);
            let output = self
                .runner
                .clone_command(&workspace, &arguments, cancel, progress, local_only)?;
            if output.status.success() {
                Ok(WriteOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            } else {
                Err(Error::Git {
                    operation: "clone",
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
        });
        let error_diagnostics = match &command {
            Ok(output) => has_error_diagnostics(&output.stderr),
            Err(Error::Git { stderr, .. }) => has_error_diagnostics(stderr),
            _ => false,
        };
        let mut report = CloneReport {
            destination,
            staging_directory: workspace,
            command,
            error_diagnostics,
            verification: None,
            publication: None,
            repository: None,
            cleanup: None,
        };
        if report.command.is_ok() {
            phase(progress, ClonePhase::Verifying);
            // Keep command success while allowing cancellation to stop verification.
            // An unverified checkout is never published.
            report.verification = Some(self.runner.step("clone", "verify_clone", || {
                self.verify_clone(&checkout, cancel)
            }));
        }
        if matches!(report.verification, Some(Ok(()))) {
            phase(progress, ClonePhase::Publishing);
            report.publication = Some(self.runner.step("clone", "publish_clone", || {
                cancel.check()?;
                plain_directory(&report.staging_directory)?;
                plain_directory(&checkout)?;
                publish(&checkout, &report.destination)
            }));
        }
        if matches!(report.publication, Some(Ok(()))) {
            phase(progress, ClonePhase::Opening);
            report.repository = Some(self.runner.step("clone", "open_clone", || {
                self.open(&report.destination, cancel)
                    .map_err(|e| phase_error(e, "clone-open", CloneFailure::OpenFailed))
            }));
            report.cleanup = Some(self.runner.step("clone", "cleanup_workspace", || {
                // Never recurse, even in a workspace we created: another process may
                // have added files. A failure leaves them available for inspection.
                fs::remove_dir(&template)
                    .map_err(|e| Error::io("remove empty clone template", e))?;
                fs::remove_dir(&report.staging_directory)
                    .map_err(|e| Error::io("remove empty clone workspace", e))
            }));
        }
        if let Some(log) = &self.runner.log {
            log.clone_outcome(&report);
        }
        Ok(report)
    }

    fn verify_clone(&self, directory: &Path, cancel: &CancellationToken) -> Result<()> {
        (|| {
            cancel.check()?;
            plain_directory(directory)?;
            plain_directory(&directory.join(".git"))?;
            let repo = self.open(directory, cancel)?;
            let root = directory
                .canonicalize()
                .map_err(|e| Error::io("resolve clone checkout", e))?;
            let expected_git = root.join(".git");
            if repo.info().worktree.as_deref() != Some(root.as_path())
                || repo.info().git_dir != expected_git
                || repo.info().common_dir != expected_git
            {
                return Err(Error::Clone(CloneFailure::VerificationFailed));
            }
            match fs::symlink_metadata(expected_git.join("objects/info/alternates")) {
                Ok(_) => return Err(Error::Clone(CloneFailure::VerificationFailed)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(Error::io("clone-verify", error)),
            }
            let status = repo.status(cancel)?;
            let branches = repo.branches(cancel)?;
            if matches!(status.head, Head::Unborn { .. }) && !branches.is_empty() {
                return Err(Error::Clone(CloneFailure::MissingDefaultBranch));
            }
            if !status.entries.is_empty() {
                return Err(Error::Clone(CloneFailure::VerificationFailed));
            }
            Ok(())
        })()
        .map_err(|e| phase_error(e, "clone-verify", CloneFailure::VerificationFailed))
    }
}

fn phase_error(error: Error, operation: &'static str, fallback: CloneFailure) -> Error {
    match error {
        Error::Git {
            code,
            stdout,
            stderr,
            ..
        } => Error::Git {
            operation,
            code,
            stdout,
            stderr,
        },
        Error::Clone(_) | Error::Cancelled | Error::TimedOut | Error::OutputLimit { .. } => error,
        Error::Io { source, .. } => Error::io(operation, source),
        _ => Error::Clone(fallback),
    }
}

fn phase(sender: Option<&SyncSender<CloneProgress>>, value: ClonePhase) {
    if let Some(sender) = sender {
        let _ = sender.try_send(CloneProgress::Phase(value));
    }
}

pub(crate) fn source_argument(source: &CloneSource) -> Result<OsString> {
    match source {
        CloneSource::Local(path) => {
            let path = path
                .canonicalize()
                .map_err(|_| Error::Clone(CloneFailure::SourceUnavailable))?;
            if !path.is_dir() {
                return Err(Error::Clone(CloneFailure::SourceUnavailable));
            }
            Ok(git_path(&path))
        }
        CloneSource::Url(url) => {
            let invalid = || Error::Clone(CloneFailure::UnsupportedSource);
            if url.chars().any(|c| c.is_control() || c.is_whitespace())
                || url.contains(['?', '#', '\\'])
            {
                return Err(invalid());
            }
            let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
                (true, rest)
            } else if let Some(rest) = url.strip_prefix("ssh://") {
                (false, rest)
            } else {
                return Err(invalid());
            };
            let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
            if authority.is_empty() || authority.contains('%') || path.is_empty() {
                return Err(invalid());
            }
            if let Some((user, host)) = authority.rsplit_once('@')
                && (https || user.is_empty() || user.contains([':', '%', '@']) || host.is_empty())
            {
                return Err(invalid());
            }
            Ok(url.into())
        }
    }
}

/// Where a clone would go, decided as `clone_repository` decides it but without
/// writing anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloneDestination {
    /// A new folder in an existing parent: the path the clone creates.
    New(PathBuf),
    /// Something is already there; clones only go into new folders.
    Exists,
    /// The parent folder does not exist.
    MissingParent,
}

/// Check a clone destination without changing anything. A name Windows refuses is an
/// error worded as for cloning.
pub fn check_clone_destination(path: &Path) -> Result<CloneDestination> {
    match destination_path(path) {
        Ok(destination) => Ok(CloneDestination::New(destination)),
        Err(Error::Clone(CloneFailure::DestinationExists)) => Ok(CloneDestination::Exists),
        Err(error) => match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(parent) if !parent.is_dir() => Ok(CloneDestination::MissingParent),
            _ => Err(error),
        },
    }
}

fn destination_path(path: &Path) -> Result<PathBuf> {
    let invalid = || Error::Clone(CloneFailure::InvalidDestination);
    // Check original spelling as Path components normalize away trailing dots.
    let text = path.as_os_str().to_string_lossy();
    let leaf_text = text.rsplit(['/', '\\']).next().ok_or_else(invalid)?;
    if leaf_text.is_empty()
        || leaf_text.ends_with(['.', ' '])
        || leaf_text
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        || matches!(leaf_text, "." | "..")
    {
        return Err(invalid());
    }
    let stem = leaf_text
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    }) {
        return Err(invalid());
    }
    let leaf = path.file_name().ok_or_else(invalid)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize().map_err(|_| invalid())?;
    if !parent.is_dir() {
        return Err(invalid());
    }
    let destination = parent.join(leaf);
    match fs::symlink_metadata(&destination) {
        Ok(_) => Err(Error::Clone(CloneFailure::DestinationExists)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(destination),
        Err(_) => Err(invalid()),
    }
}

fn create_workspace(parent: &Path) -> Result<PathBuf> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let path = parent.join(format!(
            ".rocket-clone-{}-{time:x}-{}",
            std::process::id(),
            NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io("prepare clone workspace", e)),
        }
    }
    Err(Error::Clone(CloneFailure::WorkspaceUnavailable))
}

fn plain_directory(path: &Path) -> Result<()> {
    let meta =
        fs::symlink_metadata(path).map_err(|_| Error::Clone(CloneFailure::VerificationFailed))?;
    #[cfg(windows)]
    let redirected = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT, including junctions
    };
    #[cfg(not(windows))]
    let redirected = meta.file_type().is_symlink();
    if !meta.is_dir() || redirected {
        return Err(Error::Clone(CloneFailure::VerificationFailed));
    }
    Ok(())
}

#[cfg(windows)]
fn publish(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    fn wide(value: &Path) -> Result<Vec<u16>> {
        let mut value: Vec<_> = value.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(Error::Clone(CloneFailure::InvalidDestination));
        }
        value.push(0);
        Ok(value)
    }
    let from = wide(source)?;
    let to = wide(destination)?;
    // SAFETY: both buffers are live, NUL-terminated UTF-16 strings. Flags zero
    // forbid replacement and cross-volume copy. The workspace is under the selected
    // parent volume; no std::fs::rename fallback may replace an existing target.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if fs::symlink_metadata(destination).is_ok() {
        Err(Error::Clone(CloneFailure::DestinationExists))
    } else {
        Err(Error::io("publish clone", error))
    }
}

#[cfg(not(windows))]
fn publish(_: &Path, _: &Path) -> Result<()> {
    Err(Error::Unsupported(
        "clone publication currently requires Windows".into(),
    ))
}
