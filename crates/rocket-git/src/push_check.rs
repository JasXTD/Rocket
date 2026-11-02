//! Optional application-owned checks at the last boundary before Git push starts.
use crate::{
    CancellationToken, Error, GitClient, ObjectId, Repository, Result,
    runner::{Runner, args},
};
use std::{
    ffi::OsString,
    fmt,
    path::Path,
    sync::{Arc, RwLock},
};

/// Runs on the write worker with the exact object IDs in the outgoing refspecs.
/// Returning an error or cancelling prevents that push process from starting.
/// Implementations must observe cancellation while awaiting user review. They
/// must not start another write while this operation owns its repository permit.
pub trait PushCheck: Send + Sync {
    fn check(
        &self,
        repository: &Repository,
        sources: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<()>;
}

#[derive(Clone, Default)]
pub(crate) struct Policy(Arc<RwLock<Option<Arc<dyn PushCheck>>>>);
impl fmt::Debug for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PushCheckPolicy")
    }
}
impl Policy {
    pub(crate) fn set(&self, check: Option<Arc<dyn PushCheck>>) -> Result<()> {
        *self.0.write().map_err(|_| unavailable())? = check;
        Ok(())
    }
    fn get(&self) -> Result<Option<Arc<dyn PushCheck>>> {
        Ok(self.0.read().map_err(|_| unavailable())?.clone())
    }
}
fn unavailable() -> Error {
    Error::InvalidInput("The pre-push check is unavailable. No push was started.".into())
}
fn invalid() -> Error {
    Error::InvalidInput(
        "The pre-push check requires explicit, pinned outgoing objects. No push was started."
            .into(),
    )
}

pub(crate) fn run(
    runner: &Runner,
    cwd: &Path,
    command: &[OsString],
    cancel: &CancellationToken,
) -> Result<()> {
    let position = crate::secret_guard::command_position(command);
    if command.get(position).and_then(|s| s.to_str()) != Some("push") {
        return Ok(());
    }
    let Some(check) = runner.push_check.get()? else {
        return Ok(());
    };
    let arguments = &command[position + 1..];
    let separator = arguments
        .iter()
        .position(|a| a == "--")
        .ok_or_else(invalid)?;
    if arguments.len() < separator + 3 {
        return Err(invalid());
    }
    let mut sources = Vec::new();
    for spec in &arguments[separator + 2..] {
        let (source, destination) = spec
            .to_str()
            .and_then(|s| s.split_once(':'))
            .ok_or_else(invalid)?;
        if !destination.starts_with("refs/") || destination.contains('*') {
            return Err(invalid());
        }
        if source.is_empty() {
            continue;
        } // Deletion sends no dependency tree.
        let id = ObjectId::parse(source).map_err(|_| invalid())?;
        if !sources.contains(&id) {
            sources.push(id);
        }
        if sources.len() > 256 {
            return Err(invalid());
        }
    }
    if sources.is_empty() {
        return Ok(());
    }
    // A review can remain open for a long time. Do not let changed remote/URL
    // rewriting, hooks or other Git settings silently alter the reviewed action.
    let configuration = || {
        runner
            .run(cwd, &args(&["config", "--null", "--list"]), vec![], cancel)?
            .success("pre-push configuration")
    };
    let before = configuration()?;
    let repository = GitClient::from_runner(runner.clone()).open(cwd, cancel)?;
    check.check(&repository, &sources, cancel)?;
    cancel.check()?;
    if configuration()? != before {
        return Err(Error::InvalidInput("Git configuration changed during the dependency review. Review Push again; no push was started.".into()));
    }
    if let Some(scope) = &runner.transport_scope {
        scope.validate(runner, cwd, command, cancel)?;
    }
    Ok(())
}
