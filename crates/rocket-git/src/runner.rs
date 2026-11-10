use crate::{
    CloneProgress, DiagnosticLog, Error, FetchProgress, Result,
    process_tree::ProcessTree,
    progress::{ProgressParser, ProgressSink},
};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
#[cfg(not(target_os = "macos"))]
use std::ffi::OsStr;

// An explicit empty value for --config-env, which supports '=' inside ref-derived
// configuration keys. Override any inherited value only in the child process.
pub(crate) const EMPTY_CONFIG_ENV: &str = "ROCKET_GIT_EMPTY_CONFIG";
pub(crate) const FALSE_CONFIG_ENV: &str = "ROCKET_GIT_FALSE_CONFIG";

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<CancellationState>);

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    requested_at: OnceLock<Instant>,
}
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        // Keep the first request: repeated clicks must not reset stop latency.
        self.0.requested_at.get_or_init(Instant::now);
        self.0.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    fn elapsed_since_request(&self) -> Option<Duration> {
        self.0.requested_at.get().map(Instant::elapsed)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct CommandLimits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}
impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            stdout_bytes: 32 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
        }
    }
}

pub(crate) mod http_safety;
mod privilege;

/// Receives a streamed read's standard output as it arrives. Returning false
/// ends the command: the process tree is stopped and nothing more is read.
pub(crate) type StdoutSink<'a> = &'a mut (dyn FnMut(&[u8]) -> bool + Send);

#[derive(Clone, Debug)]
pub(crate) struct Runner {
    pub executable: PathBuf,
    pub limits: CommandLimits,
    pub log: Option<DiagnosticLog>,
    pub transport_scope: Option<crate::TransportScope>,
    pub secret_protection: Arc<AtomicBool>,
    pub guard_ai_api_keys: Arc<AtomicBool>,
    pub secret_scan_bytes: Arc<std::sync::atomic::AtomicUsize>,
    pub push_check: crate::push_check::Policy,
    pub git_http_allowed: bool,
    // Holds bundled files read-only for this runner and all request/repository clones.
    pub managed_git: Option<Arc<crate::ManagedGitRuntime>>,
}
pub(crate) struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Non-overlapping process phases. A missing value means the phase was not reached.
/// These contain durations only; no command arguments, paths or output.
#[derive(Default)]
pub(crate) struct ProcessTimings {
    pub prepare: Option<Duration>,
    /// Managed Git's pre-launch recheck; absent for system Git and other tools.
    pub verify: Option<Duration>,
    pub spawn: Option<Duration>,
    pub contain: Option<Duration>,
    pub execute: Option<Duration>,
    pub drain: Option<Duration>,
    pub cancel_to_stop: Option<Duration>,
}

#[derive(Clone, Copy)]
enum ExecutionKind<'a> {
    Read,
    Write,
    TrackedWrite(&'a AtomicBool),
    CommitAction,
    InteractiveRebase(&'a crate::rebase_plan::Editor),
    Clone {
        local_only: bool,
    },
    PrivateIndex(&'a Path),
    /// A fetch that may never ask for input: no askpass program runs, Git Credential
    /// Manager may not prompt and SSH may not use an askpass.
    QuietFetch,
    /// Test connection's `ls-remote`: a read that never asks for input, like
    /// `QuietFetch`. A clone source also gets `Clone`'s protocol allowlist.
    RemoteCheck {
        clone_local_only: Option<bool>,
    },
    SigningSandbox(&'a Path),
    /// Owned disposable bare repository: no external config, attributes, hooks or transports.
    PredictionSandbox(&'a Path),
    Lfs {
        executable: &'a Path,
        config: &'a [(String, OsString)],
    },
    /// `git credential fill` outside every repository, so only global and system
    /// configuration choose the helper. GCM may prompt only when interactive.
    Credential {
        interactive: bool,
        ceiling: &'a Path,
        browser: bool,
    },
    /// A non-Git tool such as curl. Only its fixed label is logged, never its
    /// arguments, standard input or output.
    Tool {
        executable: &'a Path,
        label: &'static str,
    },
}
impl Output {
    pub(crate) fn success(self, operation: &'static str) -> Result<Vec<u8>> {
        if self.status.success() {
            Ok(self.stdout)
        } else {
            Err(Error::Git {
                operation,
                code: self.status.code(),
                stdout: self.stdout,
                stderr: self.stderr,
            })
        }
    }
}

impl Runner {
    pub(crate) fn content_guard_enabled(&self) -> bool {
        self.secret_protection.load(Ordering::Acquire)
            || self.guard_ai_api_keys.load(Ordering::Acquire)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lfs_push(
        &self,
        executable: &Path,
        cwd: &Path,
        arguments: &[OsString],
        input: Vec<u8>,
        config: &[(String, OsString)],
        progress: Option<&mpsc::SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            arguments,
            input,
            cancel,
            ExecutionKind::Lfs { executable, config },
            progress.map(ProgressSink::Lfs),
        )
    }
    pub(crate) fn lfs(
        &self,
        executable: &Path,
        cwd: &Path,
        arguments: &[OsString],
        config: &[(String, OsString)],
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            arguments,
            Vec::new(),
            cancel,
            ExecutionKind::Lfs { executable, config },
            None,
        )
    }
    // Only internal callers can select a prepared index. Inherited index paths
    // remain stripped; the live repository index is never used for preparation.
    pub(crate) fn private_index(
        &self,
        cwd: &Path,
        index: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            input,
            cancel,
            ExecutionKind::PrivateIndex(index),
            None,
        )
    }
    pub(crate) fn run(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(cwd, args, input, cancel, ExecutionKind::Read, None)
    }

    pub(crate) fn write(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(cwd, args, input, cancel, ExecutionKind::Write, None)
    }

    pub(crate) fn tracked_write(
        &self,
        cwd: &Path,
        args: &[OsString],
        cancel: &CancellationToken,
        started: &AtomicBool,
    ) -> Result<Output> {
        self.execute(cwd, args, Vec::new(), cancel, ExecutionKind::TrackedWrite(started), None)
    }

    pub(crate) fn commit_action(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(cwd, args, input, cancel, ExecutionKind::CommitAction, None)
    }

    pub(crate) fn signing_sandbox(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        empty_config: &Path,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            input,
            cancel,
            ExecutionKind::SigningSandbox(empty_config),
            None,
        )
    }

    pub(crate) fn prediction_sandbox(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        empty_config: &Path,
    ) -> Result<Output> {
        self.execute(cwd, args, input, cancel, ExecutionKind::PredictionSandbox(empty_config), None)
    }

    pub(crate) fn interactive_rebase(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        editor: &crate::rebase_plan::Editor,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            input,
            cancel,
            ExecutionKind::InteractiveRebase(editor),
            None,
        )
    }

    pub(crate) fn fetch(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        progress: Option<&mpsc::SyncSender<FetchProgress>>,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            input,
            cancel,
            ExecutionKind::Write,
            progress.map(ProgressSink::Fetch),
        )
    }

    /// [`Runner::fetch`] for automatic fetches: see `ExecutionKind::QuietFetch`.
    pub(crate) fn quiet_fetch(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(cwd, args, input, cancel, ExecutionKind::QuietFetch, None)
    }

    /// See `ExecutionKind::RemoteCheck`.
    pub(crate) fn remote_check(
        &self,
        cwd: &Path,
        args: &[OsString],
        clone_local_only: Option<bool>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            Vec::new(),
            cancel,
            ExecutionKind::RemoteCheck { clone_local_only },
            None,
        )
    }

    pub(crate) fn credential(
        &self,
        cwd: &Path,
        interactive: bool,
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            &[OsString::from("credential"), OsString::from("fill")],
            input,
            cancel,
            ExecutionKind::Credential {
                interactive,
                ceiling: cwd,
                browser: false,
            },
            None,
        )
    }

    pub(crate) fn tool(
        &self,
        executable: &Path,
        label: &'static str,
        cwd: &Path,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        self.execute(
            cwd,
            arguments,
            input,
            cancel,
            ExecutionKind::Tool { executable, label },
            None,
        )
    }

    pub(crate) fn browser_credential(&self, cwd: &Path, provider: crate::ForgeProvider, cancel: &CancellationToken) -> Result<Output> {
        let (mode, input) = match provider {
            crate::ForgeProvider::GitHub => ("credential.gitHubAuthModes=browser", b"protocol=https\nhost=github.com\n\n".as_slice()),
            crate::ForgeProvider::Bitbucket => ("credential.bitbucketAuthModes=oauth", b"protocol=https\nhost=bitbucket.org\n\n".as_slice()),
            _ => return Err(Error::InvalidInput("Unsupported browser authentication provider.".into())),
        };
        self.execute(cwd, &args(&[
            "-c", "credential.helper=", "-c", "credential.helper=manager",
            "-c", "credential.credentialStore=none",
            "-c", "credential.namespace=rocket-hosting",
            "-c", mode,
            "-c", "credential.interactive=true",
            "-c", "credential.trace=false", "-c", "credential.traceSecrets=false",
            "credential", "fill",
        ]), input.to_vec(), cancel,
            ExecutionKind::Credential {interactive:true, ceiling:cwd, browser:true}, None)
    }

    pub(crate) fn clone_command(
        &self,
        cwd: &Path,
        args: &[OsString],
        cancel: &CancellationToken,
        progress: Option<&mpsc::SyncSender<CloneProgress>>,
        local_only: bool,
    ) -> Result<Output> {
        self.execute(
            cwd,
            args,
            Vec::new(),
            cancel,
            ExecutionKind::Clone { local_only },
            progress.map(ProgressSink::Clone),
        )
    }

    /// A read whose standard output goes to `sink` as it arrives instead of being
    /// collected. `limits.stdout_bytes` bounds the total streamed. When the sink
    /// returns false the process tree is stopped and the result is `Ok` with empty
    /// stdout; the caller knows it asked to stop. A timeout or cancellation still
    /// returns its error, after the sink has seen everything that arrived.
    pub(crate) fn read_streamed(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        limits: &CommandLimits,
        sink: StdoutSink<'_>,
    ) -> Result<Output> {
        self.execute_with(cwd, args, input, cancel, ExecutionKind::Read, None, Some((sink, limits)))
    }

    /// A tool run whose standard output streams to `sink`, as `read_streamed` does for Git.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tool_streamed(
        &self,
        executable: &Path,
        label: &'static str,
        cwd: &Path,
        arguments: &[OsString],
        cancel: &CancellationToken,
        limits: &CommandLimits,
        sink: StdoutSink<'_>,
    ) -> Result<Output> {
        self.execute_with(
            cwd,
            arguments,
            Vec::new(),
            cancel,
            ExecutionKind::Tool { executable, label },
            None,
            Some((sink, limits)),
        )
    }

    fn execute(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        kind: ExecutionKind<'_>,
        progress: Option<ProgressSink<'_>>,
    ) -> Result<Output> {
        self.execute_with(cwd, args, input, cancel, kind, progress, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_with(
        &self,
        cwd: &Path,
        args: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
        kind: ExecutionKind<'_>,
        progress: Option<ProgressSink<'_>>,
        stream: Option<(StdoutSink<'_>, &CommandLimits)>,
    ) -> Result<Output> {
        let (sink, limits) = match stream {
            Some((sink, limits)) => (Some(sink), limits),
            None => (None, &self.limits),
        };
        let writing = !matches!(
            kind,
            ExecutionKind::Read
                | ExecutionKind::Credential { .. }
                | ExecutionKind::Tool { .. }
                | ExecutionKind::RemoteCheck { .. }
        );
        let helper = matches!(kind, ExecutionKind::Lfs { .. });
        let tool = matches!(kind, ExecutionKind::Tool { .. });
        let started = match kind { ExecutionKind::TrackedWrite(started) => Some(started), _ => None };
        if !helper && !tool && let Some(scope) = &self.transport_scope {
            scope.validate(self, cwd, args, cancel)?;
        }
        if helper && args.first().is_some_and(|arg| arg != "version")
            && let Some(scope) = &self.transport_scope {
            scope.configuration(self, cwd, &[], cancel)?;
        }
        if writing && !helper && !tool && self.content_guard_enabled() {
            self.step("secret-protection", "scan_before_write", || {
                crate::secret_guard::check(self, cwd, args, &input, cancel)
            })?;
        }
        if writing && !helper && !tool {
            crate::push_check::run(self, cwd, args, cancel)?;
        }
        let mut command = Command::new(match kind {
            ExecutionKind::Lfs { executable, .. } | ExecutionKind::Tool { executable, .. } => {
                executable
            }
            _ => &self.executable,
        });
        if helper || tool {
            command.current_dir(cwd);
        } else {
            command
                .current_dir(cwd)
                .args(["--no-pager", "--no-lazy-fetch", "--literal-pathspecs"])
                .args([
                    "-c",
                    "core.fsmonitor=false",
                    "-c",
                    "gc.auto=0",
                    "-c",
                    "maintenance.auto=false",
                ]);
            // Writes use Git's normal locking. A cancelled write is reconciled separately;
            // optional read locks must not accidentally become a process-wide policy.
            if !writing {
                // git diff refreshes index stat entries even with optional locks off.
                // A preview must not mutate staging metadata or trigger its own watcher.
                command
                    .args(["-c", "diff.autoRefreshIndex=false"])
                    .env("GIT_OPTIONAL_LOCKS", "0");
            } else {
                command.env_remove("GIT_OPTIONAL_LOCKS");
            }
        }
        if self.transport_scope.is_some() && !helper && !tool {
            // Invocation policy follows caller -c/--config-env options so a clone
            // or another internal caller cannot restore a configured hook path.
            let prefix = crate::transport_scope::config_prefix(args);
            command.args(&args[..prefix]);
            for (key, value) in crate::transport_scope::PROCESS_CONFIG {
                command.args(["-c", &format!("{key}={value}")]);
            }
            command.args(&args[prefix..]);
        } else {
            command.args(args);
        }
        if matches!(
            kind,
            ExecutionKind::CommitAction | ExecutionKind::InteractiveRebase(_)
        ) {
            // Continue may invoke commit without --no-edit. Use Git's shell
            // no-op editor for these reviewed operations, never an inherited
            // interactive editor. Messages and paths are not shell fragments.
            command
                .env("GIT_EDITOR", ":")
                .env("GIT_SEQUENCE_EDITOR", ":")
                .env("GIT_MERGE_AUTOEDIT", "no");
        }
        // A GUI launched from another repository must not inherit its index, config overrides,
        // pathspec mode, object store, or discovery boundary. Disk configuration remains intact.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if matches!(
                name.as_str(),
                "GIT_DIR"
                    | "GIT_WORK_TREE"
                    | "GIT_COMMON_DIR"
                    | "GIT_INDEX_FILE"
                    | "GIT_OBJECT_DIRECTORY"
                    | "GIT_ALTERNATE_OBJECT_DIRECTORIES"
                    | "GIT_SHALLOW_FILE"
                    | "GIT_NAMESPACE"
                    | "GIT_CEILING_DIRECTORIES"
                    | "GIT_DISCOVERY_ACROSS_FILESYSTEM"
                    | "GIT_CONFIG"
                    | "GIT_CONFIG_PARAMETERS"
                    | "GIT_CONFIG_COUNT"
                    | "GIT_PREFIX"
                    | "GIT_GLOB_PATHSPECS"
                    | "GIT_NOGLOB_PATHSPECS"
                    | "GIT_ICASE_PATHSPECS"
                    | "GIT_LITERAL_PATHSPECS"
                    | "GIT_EXTERNAL_DIFF"
                    | "GIT_DIFF_OPTS"
            ) || name.starts_with("GIT_CONFIG_KEY_")
                || name.starts_with("GIT_CONFIG_VALUE_")
            {
                command.env_remove(key);
            }
        }
        command
            .env(EMPTY_CONFIG_ENV, "")
            .env(FALSE_CONFIG_ENV, "false")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("LC_ALL", "C");
        if let ExecutionKind::InteractiveRebase(editor) = kind {
            editor.configure(&mut command)?;
        }
        if let ExecutionKind::Credential {
            interactive,
            ceiling,
            browser,
        } = kind
        {
            // Callers supply the neutral cwd. Git ignores that exact directory
            // when matching ceilings; stop before its parent instead, so an
            // ancestor repository cannot contribute a credential helper. Refuse
            // delimiters that Git would interpret as multiple ceiling paths.
            let ceiling = ceiling.parent().filter(|parent| parent.is_absolute())
                .filter(|parent| !parent.to_string_lossy().contains(if cfg!(windows) { ';' } else { ':' }))
                .ok_or_else(|| crate::Error::InvalidInput("Credential requests require an absolute temporary folder whose parent contains no Git path-list separator.".into()))?;
            // A browser connection never reads/writes the user's Git credential
            // store. Environment settings otherwise override our invocation config.
            if browser {
                for (key, _) in std::env::vars_os() {
                    let name=key.to_string_lossy().to_ascii_uppercase();
                    if name.starts_with("GCM_") || name.starts_with("GIT_TRACE") || matches!(name.as_str(),"GIT_CURL_VERBOSE"|"SSLKEYLOGFILE") {command.env_remove(key);}
                }
                command.env("GCM_CREDENTIAL_STORE","none")
                    .env("GCM_NAMESPACE","rocket-hosting")
                    .env("GCM_GITHUB_AUTHMODES","browser")
                    .env("GCM_BITBUCKET_AUTHMODES","oauth")
                    .env("GCM_TRACE","0").env("GCM_TRACE_SECRETS","0");
            }
            command
                .env(
                    "GCM_INTERACTIVE",
                    if interactive { "auto" } else { "never" },
                )
                .env("GIT_CEILING_DIRECTORIES", git_path(ceiling));
        }
        if let ExecutionKind::Tool { label, .. } = kind
            && label.starts_with("ssh-")
        {
            command
                .env_remove("SSH_ASKPASS")
                .env_remove("SSH_AGENT_PID")
                .env("SSH_ASKPASS_REQUIRE", "never");
            // Windows OpenSSH uses the ssh-agent service, never another socket. On Unix
            // SSH_AUTH_SOCK is how ssh-add reaches the agent at all.
            #[cfg(windows)]
            command.env_remove("SSH_AUTH_SOCK");
        }
        if tool {
            // A TLS key log would expose the session; curl honours it if inherited.
            command.env_remove("SSLKEYLOGFILE");
        }
        if let ExecutionKind::Tool {
            label: "claude", ..
        } = kind
        {
            // One commit draft: no prompt history or transcript, no CLAUDE.md files,
            // and no auto-update, telemetry or other non-essential traffic.
            command
                .env("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1")
                .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
                .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
                .env("DISABLE_AUTOUPDATER", "1");
        }
        if let ExecutionKind::PrivateIndex(index) = kind {
            command.env("GIT_INDEX_FILE", git_path(index));
        }
        if let ExecutionKind::QuietFetch | ExecutionKind::RemoteCheck { .. } = kind {
            // An empty GIT_ASKPASS also stops Git from falling back to core.askPass.
            command
                .env("GCM_INTERACTIVE", "never")
                .env("GIT_ASKPASS", "")
                .env("SSH_ASKPASS", "")
                .env("SSH_ASKPASS_REQUIRE", "never");
        }
        if let ExecutionKind::SigningSandbox(empty) = kind {
            // The sandbox gets the original repository's observed signing values
            // via -c. Its different gitdir must not activate other global includes.
            command
                .env("GIT_CONFIG_GLOBAL", git_path(empty))
                .env("GIT_CONFIG_NOSYSTEM", "1");
        }
        if let ExecutionKind::PredictionSandbox(empty) = kind {
            command.env("GIT_CONFIG_GLOBAL", git_path(empty))
                .env("GIT_CONFIG_SYSTEM", git_path(empty))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_ATTR_NOSYSTEM", "1")
                .env_remove("GIT_ATTR_SOURCE")
                .env("GIT_ALLOW_PROTOCOL", "")
                .env("GIT_NO_REPLACE_OBJECTS", "1");
        }
        if let ExecutionKind::Lfs { config, .. } = kind {
            // Invoke the installed helper directly, never a repository alias or
            // a program found in the current directory. Its child Git processes
            // receive only the reviewed invocation's config overrides.
            let policy = if self.transport_scope.is_some() { crate::transport_scope::PROCESS_CONFIG } else { &[] };
            command
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env("GIT_CONFIG_COUNT", (config.len() + policy.len()).to_string());
            for (i, (key, value)) in config.iter().enumerate() {
                command
                    .env(format!("GIT_CONFIG_KEY_{i}"), key)
                    .env(format!("GIT_CONFIG_VALUE_{i}"), value);
            }
            for (i, (key, value)) in policy.iter().enumerate() {
                let i = config.len() + i;
                command.env(format!("GIT_CONFIG_KEY_{i}"), key)
                    .env(format!("GIT_CONFIG_VALUE_{i}"), value);
            }
            let mut paths = vec![self.executable.parent().unwrap().to_path_buf()];
            if let Some(path) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&path));
            }
            command.env(
                "PATH",
                std::env::join_paths(paths)
                    .map_err(|_| Error::InvalidInput("Invalid helper search path".into()))?,
            );
            for (key, _) in std::env::vars_os() {
                let name = key.to_string_lossy().to_ascii_uppercase();
                if name.starts_with("GIT_LFS_")
                    || name.starts_with("GIT_TRACE")
                    || name == "GIT_CURL_VERBOSE"
                {
                    command.env_remove(key);
                }
            }
            if matches!(progress, Some(ProgressSink::Lfs(_))) {
                command.env("GIT_LFS_FORCE_PROGRESS", "1");
            }
        }
        if let ExecutionKind::Clone { local_only }
        | ExecutionKind::RemoteCheck {
            clone_local_only: Some(local_only),
        } = kind
        {
            // Unlike protocol.allow, this allowlist also overrides per-protocol
            // disk settings after URL rewrites. Never broaden an inherited list.
            let allowed: &[&str] = if local_only {
                &["file"]
            } else {
                &["https", "ssh"]
            };
            let inherited = std::env::var_os("GIT_ALLOW_PROTOCOL");
            let allowed = allowed
                .iter()
                .copied()
                .filter(|protocol| {
                    inherited.as_ref().is_none_or(|value| {
                        value.to_string_lossy().split(':').any(|p| p == *protocol)
                    })
                })
                .collect::<Vec<_>>()
                .join(":");
            command.env("GIT_ALLOW_PROTOCOL", allowed);
        }
        if let Some(scope) = &self.transport_scope {
            if !tool {
                for (key, _) in std::env::vars_os() {
                    if crate::transport_scope::network_environment(&key) { command.env_remove(key); }
                }
                for name in crate::transport_scope::COMMAND_ENV { command.env_remove(name); }
                command.env("GIT_ASKPASS", "").env("SSH_ASKPASS", "")
                    .env("SSH_ASKPASS_REQUIRE", "never");
                scope.configure_ssh(&mut command);
            }
            let allowed = ["file", "https", "ssh"].into_iter().filter(|protocol| {
                command.get_envs().find(|(k, _)| *k == "GIT_ALLOW_PROTOCOL")
                    .and_then(|(_, v)| v).map(|v| v.to_owned())
                    .or_else(|| std::env::var_os("GIT_ALLOW_PROTOCOL"))
                    .is_none_or(|v| v.to_string_lossy().split(':').any(|p| p == *protocol))
            }).collect::<Vec<_>>().join(":");
            command.env("GIT_ALLOW_PROTOCOL", allowed);
        }
        if !tool && !self.git_http_allowed {
            http_safety::restrict(&mut command);
        }
        if let Some(log) = &self.log {
            let mut helper_args = vec![OsString::from("lfs")];
            if helper {
                helper_args.extend_from_slice(args);
            }
            if let ExecutionKind::Tool { label, .. } = kind {
                helper_args = vec![OsString::from(label)];
            }
            log.command(
                if helper || tool { &helper_args } else { args },
                writing,
                input.len(),
                limits,
                |timings| {
                    run_process_tracked(
                        command,
                        input,
                        limits,
                        cancel,
                        progress,
                        timings,
                        started,
                        if tool { None } else { self.managed_git.as_deref() },
                        sink,
                    )
                },
            )
        } else {
            run_process_tracked(
                command,
                input,
                limits,
                cancel,
                progress,
                &mut ProcessTimings::default(),
                started,
                if tool { None } else { self.managed_git.as_deref() },
                sink,
            )
        }
    }

    pub(crate) fn observe<T>(
        &self,
        event: &'static str,
        operation: &'static str,
        run: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let started = Instant::now();
        let result = self.step(operation, event, run);
        if let Some(log) = &self.log {
            log.observation(event, operation, &result, started.elapsed());
        }
        result
    }

    pub(crate) fn step<T>(
        &self,
        operation: &'static str,
        step: &'static str,
        run: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        match &self.log {
            Some(log) => log.step(operation, step, run),
            None => run(),
        }
    }
}

// Git's config parser does not accept the Win32 verbatim prefix as a local path.
pub(crate) fn git_path(path: &Path) -> OsString {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            let mut normal = match prefix.kind() {
                Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    PathBuf::from(OsStr::new("\\\\")).join(server).join(share)
                }
                _ => return path.as_os_str().to_owned(),
            };
            for component in path.components().skip(1) {
                if let Component::Normal(part) = component {
                    normal.push(part);
                }
            }
            return normal.into_os_string();
        }
    }
    path.as_os_str().to_owned()
}

pub(crate) fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

pub(crate) fn has_error_diagnostics(stderr: &[u8]) -> bool {
    // Git can exit zero despite a hook/local metadata error. This is advisory,
    // not a protocol or proof of which component emitted the diagnostic.
    stderr
        .split(|b| *b == b'\n')
        .any(|line| line.starts_with(b"error:") || line.starts_with(b"fatal:"))
}

fn drain(
    mut reader: impl Read,
    limit: usize,
    stream: &'static str,
    faults: mpsc::Sender<Error>,
    mut progress: Option<ProgressParser<'_>>,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = faults.send(Error::io(
                    "read Git output",
                    std::io::Error::new(error.kind(), error.to_string()),
                ));
                return Err(Error::io("read Git output", error));
            }
        };
        if count == 0 {
            if let Some(parser) = &mut progress {
                parser.finish();
            }
            return Ok(output);
        }
        if count > limit.saturating_sub(output.len()) {
            let _ = faults.send(Error::OutputLimit { stream, limit });
            return Err(Error::OutputLimit { stream, limit });
        }
        output.extend_from_slice(&buffer[..count]);
        if let Some(parser) = &mut progress {
            parser.feed(&buffer[..count]);
        }
    }
}

pub(crate) fn run_process(
    command: Command,
    input: Vec<u8>,
    limits: &CommandLimits,
    cancel: &CancellationToken,
) -> Result<Output> {
    run_process_with_progress(
        command,
        input,
        limits,
        cancel,
        None,
        &mut ProcessTimings::default(),
    )
}

/// Both ordinary Git children and the Windows Lua container use this collector.
/// Containment must already be established before any process is passed here.
pub(crate) trait RunningProcess {
    fn terminate(&self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
}

struct GitProcess {
    child: std::process::Child,
    tree: ProcessTree,
}
impl RunningProcess for GitProcess {
    fn terminate(&self) -> std::io::Result<()> { self.tree.terminate() }
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> { self.child.wait() }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> { crate::process_tree::poll_exit(&mut self.child) }
}

pub(crate) struct ProcessPipes<I, O, E> {
    pub stdin: I,
    pub stdout: O,
    pub stderr: E,
}

#[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64"), all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn collect_worker_process(
    process: &mut impl RunningProcess,
    pipes: ProcessPipes<impl Write + Send, impl Read + Send, impl Read + Send>,
    input: Vec<u8>, limits: &CommandLimits, cancel: &CancellationToken,
) -> Result<Output> {
    collect_process(process, pipes, input, limits, cancel, None, &mut ProcessTimings::default(), None)
}

fn run_process_with_progress(
    command: Command,
    input: Vec<u8>,
    limits: &CommandLimits,
    cancel: &CancellationToken,
    progress: Option<ProgressSink<'_>>,
    timings: &mut ProcessTimings,
) -> Result<Output> {
    run_process_tracked(command, input, limits, cancel, progress, timings, None, None, None)
}

#[allow(clippy::too_many_arguments)]
fn run_process_tracked(
    mut command: Command,
    input: Vec<u8>,
    limits: &CommandLimits,
    cancel: &CancellationToken,
    progress: Option<ProgressSink<'_>>,
    timings: &mut ProcessTimings,
    started: Option<&AtomicBool>,
    runtime: Option<&crate::ManagedGitRuntime>,
    sink: Option<StdoutSink<'_>>,
) -> Result<Output> {
    cancel.check()?;
    privilege::check()?;
    if limits.timeout.is_zero() || limits.stdout_bytes == 0 || limits.stderr_bytes == 0 {
        return Err(Error::InvalidInput(
            "command timeout and output limits must be positive".into(),
        ));
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let phase = Instant::now();
    let starting = ProcessTree::prepare(&mut command);
    timings.prepare = Some(phase.elapsed());
    let starting = starting.map_err(|e| Error::io("start Git", e))?;
    if let Some(runtime) = runtime {
        let phase = Instant::now();
        let verified = runtime.revalidate(cancel);
        timings.verify = Some(phase.elapsed());
        verified?;
    }
    let phase = Instant::now();
    let child = command.spawn();
    timings.spawn = Some(phase.elapsed());
    let mut child = child.map_err(|e| Error::io("start Git", e))?;
    // Once spawned, even containment/cancellation failures may follow a remote
    // update. Never infer that nothing was sent from an error message alone.
    if let Some(started) = started { started.store(true, Ordering::Release); }
    let phase = Instant::now();
    let tree = ProcessTree::attach(starting, &child);
    timings.contain = Some(phase.elapsed());
    let tree = match tree {
        Ok(tree) => tree,
        Err(error) => {
            child
                .kill()
                .map_err(|e| Error::io("stop unowned Git process", e))?;
            child
                .wait()
                .map_err(|e| Error::io("reap unowned Git process", e))?;
            return Err(Error::io("own Git process tree", error));
        }
    };
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdin = child.stdin.take().expect("stdin was piped");
    collect_process(&mut GitProcess { child, tree }, ProcessPipes { stdin, stdout, stderr },
        input, limits, cancel, progress, timings, sink)
}

/// A streamed read's stdout: each chunk goes to `sink` and nothing is kept.
/// The sink returning false sets `stop`, and the caller then stops the process.
fn stream(
    mut reader: impl Read,
    limit: usize,
    faults: mpsc::Sender<Error>,
    sink: StdoutSink<'_>,
    stop: &AtomicBool,
) -> Result<Vec<u8>> {
    let mut buffer = [0u8; 8192];
    let mut total = 0usize;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = faults.send(Error::io(
                    "read Git output",
                    std::io::Error::new(error.kind(), error.to_string()),
                ));
                return Err(Error::io("read Git output", error));
            }
        };
        if count == 0 {
            return Ok(Vec::new());
        }
        if count > limit.saturating_sub(total) {
            let _ = faults.send(Error::OutputLimit { stream: "stdout", limit });
            return Err(Error::OutputLimit { stream: "stdout", limit });
        }
        total += count;
        if !sink(&buffer[..count]) {
            stop.store(true, Ordering::Release);
            return Ok(Vec::new());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_process(
    process: &mut impl RunningProcess,
    pipes: ProcessPipes<impl Write + Send, impl Read + Send, impl Read + Send>,
    input: Vec<u8>, limits: &CommandLimits, cancel: &CancellationToken,
    progress: Option<ProgressSink<'_>>, timings: &mut ProcessTimings,
    sink: Option<StdoutSink<'_>>,
) -> Result<Output> {
    let ProcessPipes { mut stdin, stdout, stderr } = pipes;
    let started = Instant::now();
    let stop = AtomicBool::new(false);
    // Scoped workers always join before returning. Draining both pipes concurrently avoids
    // the classic deadlock when Git fills stderr while the caller waits for stdout.
    thread::scope(|scope| {
        let (faults, fault_receiver) = mpsc::channel();
        let stdout_reader = scope.spawn({
            let faults = faults.clone();
            let stop = &stop;
            // Git LFS writes forced transfer progress to stdout. Git's ordinary
            // progress remains stderr-only; never parse file/blob stdout as status.
            let stdout_progress = progress.filter(|p| matches!(p, ProgressSink::Lfs(_)));
            move || match sink {
                Some(sink) => stream(stdout, limits.stdout_bytes, faults, sink, stop),
                None => drain(
                    stdout,
                    limits.stdout_bytes,
                    "stdout",
                    faults,
                    stdout_progress.map(ProgressParser::new),
                ),
            }
        });
        let stderr_reader = scope.spawn(move || {
            drain(
                stderr,
                limits.stderr_bytes,
                "stderr",
                faults,
                progress.map(ProgressParser::new),
            )
        });
        let writer = scope.spawn(move || {
            let result = stdin.write_all(&input);
            drop(stdin);
            result
        });
        let mut failure = None;
        let mut stopped = false;
        let status = loop {
            if cancel.is_cancelled() {
                failure = Some(Error::Cancelled);
            } else if started.elapsed() >= limits.timeout {
                failure = Some(Error::TimedOut);
            } else if let Ok(error) = fault_receiver.try_recv() {
                failure = Some(error);
            } else if stop.load(Ordering::Acquire) {
                // The streamed read has what it needs; nothing else is read.
                stopped = true;
            }
            if failure.is_some() || stopped {
                process.terminate()
                    .map_err(|e| Error::io("terminate Git process tree", e))?;
                break process.wait().map_err(|e| Error::io("reap Git", e))?;
            }
            match process.try_wait() {
                Ok(Some(status)) => {
                    // Helpers retaining inherited pipes must not survive their Git parent.
                    process.terminate()
                        .map_err(|e| Error::io("close Git process tree", e))?;
                    // Darwin's observation kept the PID reserved during cleanup.
                    #[cfg(target_os = "macos")]
                    {
                        let reaped = process.wait().map_err(|e| Error::io("reap Git", e))?;
                        debug_assert_eq!(status, reaped);
                        break reaped;
                    }
                    #[cfg(not(target_os = "macos"))]
                    break status;
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    process.terminate()
                        .map_err(|e| Error::io("terminate failed Git process", e))?;
                    let status = process
                        .wait()
                        .map_err(|e| Error::io("reap failed Git process", e))?;
                    failure = Some(Error::io("wait for Git", error));
                    break status;
                }
            }
        };
        timings.execute = Some(started.elapsed());
        let phase = Instant::now();
        let stdout = stdout_reader.join().map_err(|_| Error::WorkerPanicked)?;
        let stderr = stderr_reader.join().map_err(|_| Error::WorkerPanicked)?;
        let written = writer.join().map_err(|_| Error::WorkerPanicked)?;
        timings.drain = Some(phase.elapsed());
        if matches!(failure, Some(Error::Cancelled)) {
            // The owned process is reaped and all pipe workers have joined. Capture
            // this before logging, remote observation or repository refresh.
            timings.cancel_to_stop = cancel.elapsed_since_request();
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let stdout = stdout?;
        let stderr = stderr?;
        // Preserve Git's actual failure when it exits before accepting batch input.
        if status.success() {
            written.map_err(|e| Error::io("write Git input", e))?;
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    })
}

pub(crate) fn find_git() -> Result<PathBuf> {
    if let Some(bundled) = crate::managed_git::candidate()? { return Ok(bundled); }
    find_installed_git()
}

#[cfg(any(target_os = "macos", all(test, unix)))]
pub(crate) mod macos_git;

#[cfg(target_os = "macos")]
pub(crate) fn find_installed_git() -> Result<PathBuf> {
    macos_git::find(&std::env::var_os("PATH").unwrap_or_default(), std::env::var_os("DEVELOPER_DIR").as_deref())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn find_installed_git() -> Result<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&paths).filter(|p| p.is_absolute()) {
        let candidate = directory.join(OsStr::new(name));
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|e| Error::io("resolve Git executable", e));
        }
    }
    Err(Error::InvalidInput(
        "Git was not found on PATH; supply its absolute executable path".into(),
    ))
}

#[cfg(test)]
mod scope_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "runner::tests::child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("ROCKET_PROCESS_FIXTURE", mode);
        command
    }

    // This is a subprocess entry point, deliberately excluded from normal test discovery.
    // The tests below execute this same compiled binary, without downloaded helper tools.
    #[test]
    #[ignore = "subprocess fixture invoked by runner tests"]
    fn child_fixture() {
        match std::env::var("ROCKET_PROCESS_FIXTURE").unwrap().as_str() {
            "pipes" => {
                let a = thread::spawn(|| {
                    std::io::stdout()
                        .write_all(&vec![b'o'; 256 * 1024])
                        .unwrap()
                });
                let b = thread::spawn(|| {
                    std::io::stderr()
                        .write_all(&vec![b'e'; 256 * 1024])
                        .unwrap()
                });
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
                assert_eq!(input.len(), 256 * 1024);
                a.join().unwrap();
                b.join().unwrap();
            }
            "flood" => loop {
                std::io::stderr().write_all(&[b'e'; 8192]).unwrap();
            },
            // Endless stdout; a streamed read must be able to end it early.
            "stdout-flood" => loop {
                if std::io::stdout().write_all(&[b'o'; 8192]).is_err() {
                    return;
                }
            },
            // Some stdout, then silence until a time limit or Stop ends it.
            "stdout-then-wait" => {
                std::io::stdout().write_all(b"first part\n").unwrap();
                std::io::stdout().flush().unwrap();
                thread::sleep(Duration::from_secs(30));
            }
            "wait" => {
                if let Some(path) = std::env::var_os("ROCKET_FIXTURE_READY") {
                    std::fs::write(path, b"ready").unwrap();
                }
                thread::sleep(Duration::from_secs(30));
            }
            "spawn" | "spawn-and-exit" => {
                let mut child = fixture("wait");
                child
                    .stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    child.creation_flags(0x0800_0000);
                }
                #[expect(
                    clippy::zombie_processes,
                    reason = "fixture intentionally abandons a descendant to verify the runner owns and terminates it"
                )]
                let mut child = child.spawn().unwrap();
                if std::env::var("ROCKET_PROCESS_FIXTURE").unwrap() == "spawn" {
                    child.wait().unwrap();
                }
                // Dropping Child deliberately leaves a descendant and inherited pipes behind.
            }
            _ => panic!("unknown process fixture"),
        }
    }

    fn marker() -> PathBuf {
        // SystemTime can repeat between parallel tests on a coarse OS clock.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/runner-tests");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(format!(
            "{}-{}-{}.ready",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn wait_ready(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.is_file() {
            assert!(Instant::now() < deadline, "fixture did not start");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn drains_both_pipes_while_feeding_stdin() {
        let output = run_process(
            fixture("pipes"),
            vec![b'i'; 256 * 1024],
            &CommandLimits::default(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(output.status.success());
        assert!(
            output
                .stdout
                .windows(1024)
                .any(|chunk| chunk.iter().all(|&b| b == b'o'))
        );
        assert_eq!(output.stderr, vec![b'e'; 256 * 1024]);
    }

    #[test]
    fn stops_stderr_flood_at_the_limit() {
        let limits = CommandLimits {
            stderr_bytes: 1024,
            ..Default::default()
        };
        assert!(matches!(
            run_process(
                fixture("flood"),
                Vec::new(),
                &limits,
                &CancellationToken::new()
            ),
            Err(Error::OutputLimit {
                stream: "stderr",
                limit: 1024
            })
        ));
    }

    #[test]
    fn times_out_a_running_process() {
        let limits = CommandLimits {
            timeout: Duration::from_millis(300),
            ..Default::default()
        };
        assert!(matches!(
            run_process(
                fixture("wait"),
                Vec::new(),
                &limits,
                &CancellationToken::new()
            ),
            Err(Error::TimedOut)
        ));
    }

    fn streamed(command: Command, limits: &CommandLimits, cancel: &CancellationToken, sink: StdoutSink<'_>) -> Result<Output> {
        run_process_tracked(command, Vec::new(), limits, cancel, None, &mut ProcessTimings::default(), None, None, Some(sink))
    }

    #[test]
    fn streamed_read_hands_output_over_and_stops_when_the_sink_has_enough() {
        let mut seen = 0usize;
        let started = Instant::now();
        // The child is this test binary, so libtest's own header comes first.
        let output = streamed(fixture("stdout-flood"), &CommandLimits::default(), &CancellationToken::new(), &mut |chunk: &[u8]| {
            seen += chunk.iter().filter(|b| **b == b'o').count();
            seen < 100_000
        })
        .unwrap();
        assert!(seen >= 100_000, "the sink saw {seen} bytes");
        assert!(output.stdout.is_empty(), "streamed output is never collected");
        assert!(!output.status.success(), "the endless writer was stopped");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn streamed_read_keeps_what_arrived_before_a_time_limit_or_byte_cap() {
        let limits = CommandLimits { timeout: Duration::from_millis(1500), ..Default::default() };
        let mut text = Vec::new();
        let result = streamed(fixture("stdout-then-wait"), &limits, &CancellationToken::new(), &mut |chunk: &[u8]| {
            text.extend_from_slice(chunk);
            true
        });
        assert!(matches!(result, Err(Error::TimedOut)));
        assert!(text.windows(11).any(|w| w == b"first part\n"), "{}", String::from_utf8_lossy(&text));
        let limits = CommandLimits { stdout_bytes: 64 * 1024, ..Default::default() };
        let mut seen = 0usize;
        let result = streamed(fixture("stdout-flood"), &limits, &CancellationToken::new(), &mut |chunk: &[u8]| {
            seen += chunk.len();
            true
        });
        assert!(matches!(result, Err(Error::OutputLimit { stream: "stdout", .. })));
        assert!(seen <= 64 * 1024 && seen > 0);
        let cancel = CancellationToken::new();
        let mut calls = 0;
        let result = streamed(fixture("stdout-flood"), &CommandLimits::default(), &cancel, &mut |_: &[u8]| {
            calls += 1;
            if calls == 3 {
                cancel.cancel();
            }
            true
        });
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    fn tracked_spawn_distinguishes_early_refusal_from_late_timeout() {
        let started = AtomicBool::new(false);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(run_process_tracked(fixture("wait"), vec![], &CommandLimits::default(), &cancelled, None, &mut ProcessTimings::default(), Some(&started), None, None), Err(Error::Cancelled)));
        assert!(!started.load(Ordering::Acquire));
        assert!(run_process_tracked(Command::new("rocket-nonexistent-security-fixture.exe"), vec![], &CommandLimits::default(), &CancellationToken::new(), None, &mut ProcessTimings::default(), Some(&started), None, None).is_err());
        assert!(!started.load(Ordering::Acquire));
        let limits = CommandLimits { timeout: Duration::from_millis(300), ..Default::default() };
        assert!(matches!(run_process_tracked(fixture("wait"), vec![], &limits, &CancellationToken::new(), None, &mut ProcessTimings::default(), Some(&started), None, None), Err(Error::TimedOut)));
        assert!(started.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_reaps_helpers_that_hold_output_pipes() {
        let ready = marker();
        let mut command = fixture("spawn");
        command.env("ROCKET_FIXTURE_READY", &ready);
        let cancel = CancellationToken::new();
        let worker_token = cancel.clone();
        let worker = thread::spawn(move || {
            let mut timings = ProcessTimings::default();
            let result = run_process_with_progress(
                command,
                Vec::new(),
                &CommandLimits::default(),
                &worker_token,
                None,
                &mut timings,
            );
            (result, timings.cancel_to_stop)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.is_file() {
            if worker.is_finished() || Instant::now() >= deadline {
                cancel.cancel();
                match worker.join() {
                    Ok((Ok(output), _)) => panic!(
                        "fixture exited before readiness: {}; stdout: {}; stderr: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(2048)]),
                        String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(2048)]),
                    ),
                    Ok((Err(error), _)) => panic!("fixture failed before readiness: {error:?}"),
                    Err(_) => panic!("fixture worker panicked before readiness"),
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        let started = Instant::now();
        cancel.cancel();
        let requested = *cancel.0.requested_at.get().unwrap();
        cancel.cancel();
        assert_eq!(cancel.0.requested_at.get(), Some(&requested));
        let (result, latency) = worker.join().unwrap();
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(latency.is_some_and(|elapsed| elapsed <= started.elapsed()));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "descendant-held pipes delayed cancellation"
        );
        std::fs::remove_file(ready).unwrap();
    }

    #[test]
    fn pre_cancelled_command_does_not_claim_a_process_stop_time() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut timings = ProcessTimings::default();
        let result = run_process_with_progress(
            fixture("pipes"),
            Vec::new(),
            &CommandLimits::default(),
            &cancel,
            None,
            &mut timings,
        );
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(timings.cancel_to_stop, None);
        assert_eq!(timings.verify, None);
        assert_eq!(timings.spawn, None);
        assert_eq!(timings.execute, None);
    }

    #[test]
    fn failed_spawn_records_only_attempted_phases() {
        let mut timings = ProcessTimings::default();
        let result = run_process_with_progress(
            Command::new(marker().join("missing-executable")),
            Vec::new(),
            &CommandLimits::default(),
            &CancellationToken::new(),
            None,
            &mut timings,
        );
        assert!(matches!(
            result,
            Err(Error::Io {
                action: "start Git",
                ..
            })
        ));
        assert!(timings.prepare.is_some());
        assert!(timings.verify.is_none());
        assert!(timings.spawn.is_some());
        assert!(timings.contain.is_none());
        assert!(timings.execute.is_none());
        assert!(timings.drain.is_none());
        assert!(timings.cancel_to_stop.is_none());
    }

    #[test]
    fn exiting_parent_does_not_leave_a_helper_holding_pipes() {
        let started = Instant::now();
        let output = run_process(
            fixture("spawn-and-exit"),
            Vec::new(),
            &CommandLimits::default(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn dropping_background_task_cancels_active_process() {
        let ready = marker();
        let mut command = fixture("wait");
        command.env("ROCKET_FIXTURE_READY", &ready);
        let (sender, receiver) = mpsc::channel();
        let task = crate::ReadTask::spawn(move |cancel| {
            let result = run_process(command, Vec::new(), &CommandLimits::default(), &cancel);
            sender
                .send(matches!(result, Err(Error::Cancelled)))
                .unwrap();
            result.map(|_| ())
        })
        .unwrap();
        wait_ready(&ready);
        drop(task);
        assert!(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
        std::fs::remove_file(ready).unwrap();
    }
}
