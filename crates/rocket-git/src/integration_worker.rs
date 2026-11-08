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
            Err(unsupported_platform())
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
        // An unsupported host cannot become safe by binding a worker hash. Refuse
        // before inspecting the worker or suggesting a Windows/Linux rebuild.
        #[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
        {
            let _ = (&self.path, &self.sha256, input, limits);
            Err(unsupported_platform())
        }
        #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
        {
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
        }
    }
}

#[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
fn unsupported_platform() -> Error {
    Error::Unsupported(
        if cfg!(target_os = "macos") {
            "integration_worker_sandbox: Lua extensions are unavailable in the macOS preview until its worker sandbox is implemented and verified. No script was run."
        } else {
            "integration_worker_sandbox: Lua execution needs the worker sandbox of Windows or Linux x86_64; no unrestricted fallback is provided."
        }
        .into(),
    )
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    #[test]
    fn cancellation_precedes_worker_access() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let worker = IntegrationWorker::new(PathBuf::from("absent-worker"), "");
        assert!(matches!(
            worker.run(Vec::new(), &CommandLimits::default(), &cancel),
            Err(Error::Cancelled)
        ));
    }

    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    #[test]
    fn supported_platform_still_requires_a_bound_worker() {
        for digest in ["".to_owned(), "g".repeat(64), "a".repeat(63)] {
            let worker = IntegrationWorker::new(PathBuf::from("absent-worker"), &digest);
            let error = worker
                .run(
                    Vec::new(),
                    &CommandLimits::default(),
                    &CancellationToken::new(),
                )
                .err()
                .expect("unbound workers must be refused before file access");
            assert!(
                matches!(error, Error::InvalidInput(ref message) if message.starts_with("integration_worker_unbound:"))
            );
        }
    }

    #[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
    #[test]
    fn unsupported_platform_refuses_bound_and_unbound_workers() {
        for digest in ["".to_owned(), "a".repeat(64)] {
            let worker = IntegrationWorker::new(PathBuf::from("absent-worker"), &digest);
            assert!(matches!(
                worker.run(Vec::new(), &CommandLimits::default(), &CancellationToken::new()),
                Err(Error::Unsupported(message)) if message.starts_with("integration_worker_sandbox:")
            ));
        }
        assert!(matches!(
            IntegrationWorker::enter_isolation(),
            Err(Error::Unsupported(_))
        ));
    }
}
