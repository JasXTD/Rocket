//! Worker identity is supplied by trusted host code, never a package or sidecar.
use crate::{CancellationToken, CommandLimits, Error, Result, runner::Output};
use std::path::PathBuf;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux;
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

    /// Worker entry points call this first, before reading or compiling untrusted input.
    /// On Windows the launcher starts the worker in its container, and this checks that
    /// it is there. On Linux x86_64 the worker locks itself down here with Landlock and
    /// seccomp. That can't be undone, so nothing but the worker's entry point may call it;
    /// it refuses a process with more than one thread.
    pub fn enter_isolation() -> Result<()> {
        #[cfg(windows)]
        {
            windows::check_current()
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            linux::enter_current()
        }
        #[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
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
            let script = if cfg!(windows) {
                "tools/Build-Local.ps1"
            } else {
                "tools/build-local.sh"
            };
            return Err(Error::InvalidInput(format!(
                "integration_worker_unbound: This build has no valid embedded Lua worker hash. Build the complete bundle with {script}."
            )));
        }
        #[cfg(windows)]
        {
            windows::run(self, input, limits, cancel)
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            linux::run(self, input, limits, cancel)
        }
        #[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
        {
            let _ = (&self.path, input, limits);
            Err(Error::Unsupported("integration_worker_sandbox: Lua execution needs the worker sandbox of Windows or Linux x86_64; no unrestricted fallback is provided.".into()))
        }
    }
}
