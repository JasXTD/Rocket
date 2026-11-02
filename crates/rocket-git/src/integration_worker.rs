//! Worker identity is supplied by trusted host code, never a package or sidecar.
use crate::{CancellationToken, CommandLimits, Error, Result, runner::Output};
use std::path::PathBuf;

#[cfg(windows)]
mod windows;

/// The worker file and its SHA-256 embedded into the native host at build time.
/// This binds an unsigned bundle; it does not authenticate a replaced host itself.
#[derive(Clone, Debug)]
pub struct IntegrationWorker {
    path: PathBuf,
    sha256: String,
}

impl IntegrationWorker {
    pub fn new(path: PathBuf, embedded_sha256: &str) -> Self {
        Self {
            path,
            sha256: embedded_sha256.to_ascii_lowercase(),
        }
    }

    /// Worker entry points call this before reading or compiling untrusted input.
    pub fn check_isolation() -> Result<()> {
        #[cfg(windows)]
        {
            windows::check_current()
        }
        #[cfg(not(windows))]
        {
            Err(Error::Unsupported(
                "Lua worker isolation is unavailable on this platform".into(),
            ))
        }
    }

    pub(crate) fn run(
        &self,
        input: Vec<u8>,
        limits: &CommandLimits,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        cancel.check()?;
        crate::ensure_unprivileged_process()?;
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::InvalidInput("integration_worker_unbound: This build has no valid embedded Lua worker hash. Build the complete bundle with tools/Build-Local.ps1.".into()));
        }
        #[cfg(windows)]
        {
            windows::run(self, input, limits, cancel)
        }
        #[cfg(not(windows))]
        {
            let _ = (&self.path, input, limits);
            Err(Error::Unsupported("integration_worker_sandbox: Lua execution requires the Windows worker sandbox; no unrestricted fallback is provided.".into()))
        }
    }
}
