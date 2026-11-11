//! Bounded local diagnostics. Payloads, paths, arguments, and error text stay out of logs.
use crate::{
    CommandLimits, Error, Result,
    runner::{Output, ProcessTimings},
};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions, TryLockError},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILES: usize = 5;
mod overflow;
const MAX_BYTES: u64 = 1024 * 1024;
// Allow brief scheduler/filesystem stalls during another process's rotation,
// while still giving up if a live process holds the lock indefinitely.
const LOCK_WAIT: Duration = Duration::from_millis(500);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// A cloneable local JSON-lines logger. Five files of at most 1 MiB share one
/// directory. Contention uses a durable, bounded 256-record overflow queue.
/// Cooperating processes serialize rotation and recovery through an OS lock.
/// Logging is best effort on I/O failure or exhausted capacity; inspect [`Self::health`].
#[derive(Clone, Debug)]
pub struct DiagnosticLog {
    inner: Arc<Inner>,
    request: u64,
}

#[derive(Debug)]
struct Inner {
    directory: PathBuf,
    session: String,
    next_id: AtomicU64,
    state: Mutex<State>,
    // UI/MCP health reads must never wait for filesystem work under `state`.
    health: Mutex<LogHealth>,
    max_bytes: u64,
}

#[derive(Debug, Default)]
struct State {
    sequence: u64,
    // Once an append reaches durable overflow, later events try the lock but
    // do not each repeat the full wait. A successful append clears this state.
    contended: bool,
}

/// Cumulative health for this logger and its clones. No raw error messages are retained.
#[derive(Clone, Debug, Default)]
pub struct LogHealth {
    pub dropped_events: u64,
    /// Cumulative records durably saved outside the contended rotation lock.
    pub overflow_events: u64,
    pub last_error_kind: Option<io::ErrorKind>,
    pub last_os_error: Option<i32>,
}

/// Native MCP timings only. Labels are allowlisted again by the logger.
pub struct McpRequestMetrics<'a> {
    pub native_request: u64,
    pub tool: &'a str,
    pub phase: &'a str,
    pub outcome: &'a str,
    pub elapsed_ms: u64,
    pub queue_ms: u64,
    pub validation_ms: u64,
    pub phase_ms: u64,
    pub input_delivered: bool,
}

/// Fixed labels for the native MCP input guard. No repository-derived text.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum InputValidationStep {
    Configuration = 0,
    Remotes = 1,
    Destination = 2,
    Recheck = 3,
    Fixture = 4,
}
impl InputValidationStep {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "read_configuration",
            Self::Remotes => "validate_remotes",
            Self::Destination => "resolve_destination",
            Self::Recheck => "recheck_configuration",
            Self::Fixture => "validate_fixture",
        }
    }
}

impl DiagnosticLog {
    /// Fixed, bounded retention paths, including durable contention overflow.
    /// Readers deduplicate by (session, seq) after crash recovery.
    pub fn retained_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files: Vec<_> = (0..FILES).map(|index| path(directory, index)).collect();
        if let Some(path) = overflow::directory(directory, false)? {
            let mut entries = fs::read_dir(path)?;
            if entries.next().transpose()?.is_some() {
                files.extend(overflow::paths(directory));
            }
        }
        Ok(files)
    }
    /// Use a dedicated, trusted local directory. Only `rocket.jsonl`,
    /// `rocket.1.jsonl` through `rocket.4.jsonl`, `rocket.lock`, and the fixed
    /// slots in `rocket-overflow/` are managed (at most 9 MiB total).
    /// Do not remove/replace the lock file while any logger is running.
    pub fn open(directory: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_limit(directory.as_ref(), MAX_BYTES)
    }

    fn open_with_limit(directory: &Path, max_bytes: u64) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let directory = directory.canonicalize()?;
        let log = Self {
            inner: Arc::new(Inner {
                directory,
                session: format!(
                    "{:x}-{:x}-{:x}",
                    epoch_nanos(),
                    std::process::id(),
                    NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
                ),
                next_id: AtomicU64::new(1),
                state: Mutex::new(State::default()),
                health: Mutex::new(LogHealth::default()),
                max_bytes,
            }),
            request: 0,
        };
        // Report initialization errors to the caller; subsequent errors are health
        // observations, never replacements for the outcome of a Git mutation.
        log.emit(
            "session_start",
            "info",
            &format!(
                ",\"version\":\"{}\",\"os\":\"{}\",\"arch\":\"{}\"",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )?;
        Ok(log)
    }

    pub fn directory(&self) -> &Path {
        &self.inner.directory
    }
    pub fn session_id(&self) -> &str {
        &self.inner.session
    }
    /// Health of completed appends; reading it does not wait for an in-flight write.
    pub fn health(&self) -> LogHealth {
        self.inner
            .health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Correlate an application request, including errors before Git starts and
    /// parser failures. Pass the supplied logger to `GitClient::*_with_log`.
    /// Operation names use Rocket's fixed CLI vocabulary; unknown names become
    /// `other`. This deliberately does not accept arbitrary user text to log.
    /// The closure's result is returned unchanged. For a write report, its inner
    /// command result and independent refresh outcomes are also logged by the backend.
    pub fn request<T>(&self, operation: &str, run: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let context = Self {
            inner: self.inner.clone(),
            request: self.next_id(),
        };
        let operation = operation_label(operation);
        let span = Span::start(&context, "request", operation, "");
        let result = run(&context);
        span.finish(&result, "");
        result
    }

    /// Fixed numeric process evidence; never accepts tool text or key paths.
    pub fn ssh_console_exit(&self, exit: Option<u32>, elapsed: Duration, stopped: Option<Duration>) {
        self.record("ssh_console_exit", "info", &format!(
            ",\"exit_code\":{},\"elapsed_ms\":{},\"cancel_to_stop_us\":{}",
            exit.map_or("null".into(),|v|v.to_string()),elapsed.as_millis(),
            stopped.map_or("null".into(),|v|v.as_micros().to_string())));
    }
    /// Time one allowlisted input-validation step without recording its payload.
    pub fn input_validation_step<T>(
        &self,
        step: InputValidationStep,
        run: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.step("mcp-input-validation", step.code(), run)
    }

    /// Does not accept request arguments, control IDs, client IDs or error text.
    pub fn mcp_request(&self, m: &McpRequestMetrics<'_>) {
        let tool = match m.tool {
            "rocket_open"
            | "rocket_close"
            | "rocket_state"
            | "rocket_ai_conflict_list"
            | "rocket_ai_conflict_get"
            | "rocket_ai_conflict_propose"
            | "rocket_click"
            | "rocket_shortcut"
            | "rocket_drag"
            | "rocket_type"
            | "rocket_scroll"
            | "rocket_resize"
            | "rocket_capture"
            | "rocket_review_click"
            | "rocket_review_resize"
            | "rocket_review_capture"
            | "rocket_logs"
            | "rocket_wait"
            | "rocket_escape"
            | "rocket_cancel_input"
            | "rocket_terminal_enter"
            | "rocket_terminal_probe" => m.tool,
            _ => "other",
        };
        let phase = match m.phase {
            "queued" | "ui" | "guard" | "validation" | "input" | "delivered" => m.phase,
            _ => "other",
        };
        let outcome = match m.outcome {
            "ok" | "refused" | "cancelled" | "abandoned" => m.outcome,
            _ => "other",
        };
        let context = Self {
            inner: self.inner.clone(),
            request: self.next_id(),
        };
        context.record("mcp_request", if outcome == "ok" { "info" } else { "warn" }, &format!(
            ",\"operation\":\"mcp-ui\",\"native_request\":{},\"tool\":\"{tool}\",\"phase\":\"{phase}\",\"outcome\":\"{outcome}\",\"elapsed_ms\":{},\"queue_ms\":{},\"validation_ms\":{},\"phase_ms\":{},\"input_delivered\":{}",
            m.native_request, m.elapsed_ms, m.queue_ms, m.validation_ms, m.phase_ms, m.input_delivered));
    }

    pub(crate) fn command(
        &self,
        arguments: &[OsString],
        writing: bool,
        input_bytes: usize,
        limits: &CommandLimits,
        run: impl FnOnce(&mut ProcessTimings) -> Result<Output>,
    ) -> Result<Output> {
        let detail = format!(
            ",\"kind\":\"{}\",\"stdin_bytes\":{input_bytes},\"timeout_ms\":{},\"stdout_limit\":{},\"stderr_limit\":{}",
            if writing { "write" } else { "read" },
            limits.timeout.as_millis(),
            limits.stdout_bytes,
            limits.stderr_bytes
        );
        let span = Span::start(self, "git", command_label(arguments), &detail);
        let mut timings = ProcessTimings::default();
        let result = run(&mut timings);
        // A nonzero Git exit can be an expected probe (e.g. an absent ref).
        // Record it neutrally here; the request/write result determines failure.
        let mut detail = match &result {
            Ok(output) => format!(
                ",\"exit_code\":{},\"stdout_bytes\":{},\"stderr_bytes\":{}",
                optional_number(output.status.code()),
                output.stdout.len(),
                output.stderr.len()
            ),
            Err(Error::Cancelled) => timings.cancel_to_stop.map_or_else(String::new, |elapsed| {
                format!(",\"cancel_to_stop_us\":{}", elapsed.as_micros())
            }),
            Err(_) => String::new(),
        };
        for (phase, elapsed) in [
            ("prepare", timings.prepare),
            ("verify", timings.verify),
            ("spawn", timings.spawn),
            ("contain", timings.contain),
            ("execute", timings.execute),
            ("drain", timings.drain),
        ] {
            if let Some(elapsed) = elapsed {
                detail.push_str(&format!(",\"{phase}_us\":{}", elapsed.as_micros()));
            }
        }
        span.finish_fields(
            if result.is_ok() {
                ",\"outcome\":\"exited\"".into()
            } else {
                outcome(&result)
            },
            result.is_err(),
            &detail,
        );
        result
    }

    pub(crate) fn observation<T>(
        &self,
        event: &'static str,
        operation: &'static str,
        result: &Result<T>,
        elapsed: Duration,
    ) {
        self.record(
            event,
            if result.is_ok() { "info" } else { "warn" },
            &format!(
                ",\"operation\":\"{operation}\",\"elapsed_ms\":{}{}",
                elapsed.as_millis(),
                outcome(result)
            ),
        );
    }

    pub(crate) fn atomic_push_result(
        &self,
        state: crate::AtomicPushRemoteState,
        count: usize,
        command: bool,
        verified: bool,
    ) {
        self.record("atomic_push_result",if command&&verified{"info"}else{"warn"},&format!(",\"operation\":\"atomic-push\",\"remote_outcome\":\"{}\",\"branches\":{count},\"command_succeeded\":{command},\"verified\":{verified}",state.code()));
    }

    pub(crate) fn force_push_review(&self, removed: u64, added: u64) {
        self.record("force_push_review", "info", &format!(",\"operation\":\"force-push-preview\",\"removed_commits\":{removed},\"added_commits\":{added}"));
    }
    pub(crate) fn replay_prediction(&self, operation: &str, prediction: &crate::CommitPrediction) {
        let operation = operation_label(operation);
        self.record("commit_prediction", "info", &format!(
            ",\"operation\":\"{operation}\",\"outcome\":\"{}\",\"checked\":{},\"total\":{},\"conflict_paths\":{},\"reason\":\"{}\"",
            prediction.kind.code(), prediction.checked, prediction.total, prediction.paths.len(),
            prediction.reason.map_or("none", crate::PredictionReason::code)));
    }
    pub(crate) fn force_push_result(
        &self,
        remote: crate::ForcePushRemoteState,
        command_ok: bool,
        verified: bool,
        diagnostics: bool,
    ) {
        self.record("force_push_result", if command_ok && verified && !diagnostics { "info" } else { "warn" },
            &format!(",\"operation\":\"force-push\",\"remote_outcome\":\"{}\",\"command_succeeded\":{command_ok},\"verified\":{verified},\"error_diagnostics\":{diagnostics}", remote.code()));
    }
    pub(crate) fn lfs_upload_result(
        &self,
        outcome: crate::LfsUploadOutcome,
        objects: usize,
        bytes: u64,
    ) {
        self.record("lfs_upload_result", if outcome == crate::LfsUploadOutcome::ConfirmedByHelper { "info" } else { "warn" },
            &format!(",\"operation\":\"lfs-push\",\"remote_outcome\":\"{}\",\"objects\":{objects},\"object_bytes\":{bytes}", outcome.code()));
    }
    pub(crate) fn lfs_lock_result(
        &self,
        operation: &'static str,
        outcome: crate::LfsLockOutcome,
        local: bool,
        refreshed: bool,
        verified: bool,
    ) {
        self.record("lfs_lock_result",if outcome==crate::LfsLockOutcome::ConfirmedByHelper && local && refreshed && verified {"info"}else{"warn"},
            &format!(",\"operation\":\"{operation}\",\"remote_outcome\":\"{}\",\"local_verified\":{local},\"refreshed\":{refreshed},\"verified\":{verified}",outcome.code()));
    }

    pub(crate) fn keep_local_result(&self,operation:&'static str,published:Option<bool>,rules_saved:bool,files:usize,success:bool) {
        let publication=match published{Some(true)=>"confirmed",Some(false)=>"not_published",None=>"unknown"};
        self.record("keep_local_result",if success{"info"}else{"warn"},&format!(",\"operation\":\"{operation}\",\"publication\":\"{publication}\",\"rules_saved\":{rules_saved},\"files\":{files},\"success\":{success}"));
    }

    pub(crate) fn patch_series_result(&self, commits: usize, published: bool, retained: bool, success: bool) {
        self.record("patch_series_result", if success { "info" } else { "warn" }, &format!(",\"operation\":\"patch-series-export\",\"commits\":{commits},\"published\":{published},\"retained_preparation\":{retained},\"success\":{success}"));
    }

    pub(crate) fn mail_import_result(&self, operation: &'static str, pending: Option<bool>, backup: bool, recorded: bool, success: bool) {
        let pending=match pending {Some(true)=>"paused",Some(false)=>"idle",None=>"unknown"};
        self.record("mail_import_result",if success&&recorded{"info"}else{"warn"},&format!(",\"operation\":\"{operation}\",\"operation_state\":\"{pending}\",\"resolution_backup\":{backup},\"recovery_recorded\":{recorded},\"success\":{success}"));
    }

    pub(crate) fn ssh_discovery_result(&self, keys: usize, skipped: usize, truncated: bool) {
        self.record("ssh_discovery_result", if skipped > 0 || truncated { "warn" } else { "info" },
            &format!(",\"operation\":\"ssh-discover\",\"keys\":{keys},\"skipped\":{skipped},\"truncated\":{truncated}"));
    }

    /// Internal steps use only labels defined in Rocket's source. Start records
    /// make a blocked queue/helper or interrupted refresh visible before it ends.
    pub(crate) fn step<T>(
        &self,
        operation: &'static str,
        step: &'static str,
        run: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let detail = format!(",\"step\":\"{step}\"");
        let span = Span::start(self, "step", operation, &detail);
        let result = run();
        span.finish(&result, &detail);
        result
    }
    pub(crate) fn bisect_test(&self, test: &crate::BisectTestResult, count: usize) {
        let outcome = match test.outcome {
            crate::BisectTestOutcome::Running => "running",
            crate::BisectTestOutcome::Good => "good",
            crate::BisectTestOutcome::Bad => "bad",
            crate::BisectTestOutcome::Skip => "skip",
            crate::BisectTestOutcome::Cancelled => "cancelled",
            crate::BisectTestOutcome::TimedOut => "timeout",
            crate::BisectTestOutcome::Failed => "failed",
            crate::BisectTestOutcome::StateChanged => "changed",
        };
        let exit = test.exit_code.map_or("null".into(), |n| n.to_string());
        self.record("bisect_test_end", "info", &format!(",\"operation\":\"bisect-auto\",\"test\":{count},\"outcome\":\"{outcome}\",\"exit_code\":{exit},\"elapsed_ms\":{},\"truncated\":{}", test.elapsed_ms, test.truncated));
    }
    pub(crate) fn patch_outcome(
        &self,
        operation: &'static str,
        total: usize,
        changed: usize,
        backups: usize,
        complete: bool,
        verified: bool,
    ) {
        self.record(
            "patch_outcome",
            if complete && verified { "info" } else { "warn" },
            &format!(",\"operation\":\"{}\",\"total_files\":{total},\"changed_files\":{changed},\"recovery_copies\":{backups},\"complete\":{complete},\"verified\":{verified}", operation_label(operation)),
        );
    }

    pub(crate) fn submodule_action_outcome(
        &self,
        operation: &'static str,
        steps: &[&'static str],
        complete: bool,
        verified: bool,
    ) {
        // Labels originate only in Rocket, never in paths, config or Git output.
        let steps = steps
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",");
        self.record(
            "submodule_action_outcome",
            if complete && verified { "info" } else { "warn" },
            &format!(
                ",\"operation\":\"{}\",\"completed_steps\":[{steps}],\"complete\":{complete},\"verified\":{verified}", operation_label(operation)
            ),
        );
    }

    pub(crate) fn git_version(&self, major: u32, minor: u32, patch: Option<u32>) {
        // Vendor suffixes and arbitrary version-output bytes never enter the log.
        self.record(
            "git_version",
            "info",
            &format!(
                ",\"major\":{major},\"minor\":{minor},\"patch\":{}",
                patch.map_or_else(|| "null".into(), |value| value.to_string())
            ),
        );
    }

    /// Counts only: messages and command text are user data and never logged.
    pub(crate) fn rebase_plan(&self, entries: &[crate::RebasePlanEntry]) {
        use crate::RebaseInstruction::*;
        let count = |action| {
            entries
                .iter()
                .filter(|entry| entry.instruction == action)
                .count()
        };
        let commands = entries.iter().filter(|e| e.exec.is_some()).count();
        let stops = entries.iter().filter(|e| e.stop_after).count();
        self.record("rebase_plan", "info", &format!(",\"operation\":\"interactive-rebase\",\"commits\":{},\"pick\":{},\"reword\":{},\"edit\":{},\"squash\":{},\"fixup\":{},\"drop\":{},\"commands\":{commands},\"stops\":{stops}",entries.len(),count(Pick),count(Reword),count(Edit),count(Squash),count(Fixup),count(Drop)));
    }

    pub(crate) fn push_outcome(
        &self,
        remote: crate::PushRemoteOutcome,
        upstream: bool,
        error_diagnostics: bool,
    ) {
        self.record("push_outcome", if remote == crate::PushRemoteOutcome::Confirmed && !error_diagnostics { "info" } else { "warn" },
            &format!(",\"operation\":\"push\",\"remote_outcome\":\"{}\",\"upstream_requested\":{upstream},\"error_diagnostics\":{error_diagnostics}", remote.code()));
    }

    pub(crate) fn github_publish_outcome(&self, report: &crate::GitHubPublishReport) {
        let push=report.push.as_ref().map_or("not_attempted", |p|p.remote_outcome.code());
        let errors=report.error.is_some() || report.push.as_ref().is_some_and(|p|p.result.is_err() || p.error_diagnostics
            || p.update.as_ref().is_some_and(Result::is_err)
            || p.upstream.as_ref().is_some_and(Result::is_err) || p.branches.is_err()
            || p.status.as_ref().is_some_and(Result::is_err));
        self.record("github_publish_outcome", if push=="confirmed_success" && !errors {"info"} else {"warn"},
            &format!(",\"operation\":\"github-publish\",\"creation\":\"{}\",\"remote_added\":{},\"remote_outcome\":\"{push}\",\"has_errors\":{errors}",report.creation.code(),report.remote_added));
    }

    pub(crate) fn commit_action_outcome(
        &self,
        operation: &'static str,
        failure: Option<crate::CommitFailure>,
        state: &Result<crate::OperationState>,
    ) {
        let pending = state
            .as_ref()
            .map_or("unknown", |s| if s.is_idle() { "idle" } else { "pending" });
        let code = failure.map_or("none", crate::CommitFailure::code);
        self.record("commit_action_outcome", if failure.is_some() { "warn" } else { "info" },
            &format!(",\"operation\":\"{operation}\",\"failure_code\":\"{code}\",\"operation_state\":\"{pending}\""));
    }

    pub(crate) fn pull_outcome(
        &self,
        fetch: &Result<crate::FetchOutput>,
        update: &Option<Result<crate::PullUpdate>>,
    ) {
        let fetched = if fetch.is_ok() { "succeeded" } else { "failed" };
        let updated = match update {
            Some(Ok(_)) => "succeeded",
            Some(Err(_)) => "failed",
            None => "skipped",
        };
        let (kind, verified) = match update {
            Some(Ok(result)) => (
                match result.kind {
                    crate::PullUpdateKind::FastForward => "fast_forward",
                    crate::PullUpdateKind::UpToDate => "up_to_date",
                },
                if result.verification.is_ok() {
                    "ok"
                } else {
                    "failed"
                },
            ),
            _ => ("none", "not_available"),
        };
        let error_diagnostics = match update {
            Some(Ok(result)) => result.error_diagnostics,
            Some(Err(Error::Git { stderr, .. })) => crate::runner::has_error_diagnostics(stderr),
            _ => false,
        };
        self.record("pull_outcome", if verified == "ok" && !error_diagnostics { "info" } else { "warn" },
            &format!(",\"operation\":\"pull\",\"fetch_outcome\":\"{fetched}\",\"update_outcome\":\"{updated}\",\"update_kind\":\"{kind}\",\"verification\":\"{verified}\",\"error_diagnostics\":{error_diagnostics}"));
    }

    pub(crate) fn clone_outcome(&self, report: &crate::CloneReport) {
        fn phase<T>(value: &Option<Result<T>>) -> &'static str {
            match value {
                Some(Ok(_)) => "succeeded",
                Some(Err(_)) => "failed",
                None => "skipped",
            }
        }
        let command = if report.command.is_ok() {
            "succeeded"
        } else {
            "failed"
        };
        let verified = phase(&report.verification);
        let published = phase(&report.publication);
        let opened = phase(&report.repository);
        let cleaned = phase(&report.cleanup);
        let error_diagnostics = report.error_diagnostics;
        self.record("clone_outcome", if published == "succeeded" && opened == "succeeded" && cleaned == "succeeded" && !error_diagnostics { "info" } else { "warn" },
            &format!(",\"operation\":\"clone\",\"clone_command\":\"{command}\",\"verification\":\"{verified}\",\"publication\":\"{published}\",\"reopen\":\"{opened}\",\"cleanup\":\"{cleaned}\",\"error_diagnostics\":{error_diagnostics}"));
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn record(&self, event: &str, level: &str, detail: &str) {
        // emit records every failure in health; callers never lose Git results.
        let _ = self.emit(event, level, detail);
    }

    fn emit(&self, event: &str, level: &str, detail: &str) -> io::Result<()> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        // Every string here is generated internally or drawn from a fixed allowlist.
        // Never feed an Error's Display/Debug, argv, or repository bytes to this formatter.
        let record = format!(
            "{{\"schema\":1,\"time_unix_ms\":{},\"pid\":{},\"session\":\"{}\",\"seq\":{},\"request\":{},\"dropped_before\":{},\"level\":\"{level}\",\"event\":\"{event}\"{detail}}}\n",
            epoch_nanos() / 1_000_000,
            std::process::id(),
            self.inner.session,
            state.sequence,
            self.request,
            self.health().dropped_events
        );
        let result = append(
            &self.inner.directory,
            record.as_bytes(),
            self.inner.max_bytes,
            !state.contended,
        );
        // Publish counters only after I/O. Never hold this short-lived health
        // lock while waiting for the writer lock or touching the filesystem.
        let mut health = self.inner.health.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = &result {
            health.dropped_events = health.dropped_events.saturating_add(1);
            health.last_error_kind = Some(error.kind());
            health.last_os_error = error.raw_os_error();
        }
        if let Ok(overflow) = &result {
            state.contended = *overflow;
            if *overflow {
                health.overflow_events = health.overflow_events.saturating_add(1);
            }
        }
        result.map(|_| ())
    }
}

struct Span<'a> {
    log: &'a DiagnosticLog,
    event: &'static str,
    operation: &'static str,
    id: u64,
    started: Instant,
    finished: bool,
    start_detail: String,
}
impl<'a> Span<'a> {
    fn start(
        log: &'a DiagnosticLog,
        event: &'static str,
        operation: &'static str,
        detail: &str,
    ) -> Self {
        let span = Self {
            log,
            event,
            operation,
            id: log.next_id(),
            started: Instant::now(),
            finished: false,
            start_detail: detail.to_owned(),
        };
        log.record(
            &format!("{event}_start"),
            "info",
            &format!(",\"id\":{},\"operation\":\"{operation}\"{detail}", span.id),
        );
        span
    }
    fn finish<T>(self, result: &Result<T>, detail: &str) {
        self.finish_fields(outcome(result), result.is_err(), detail);
    }
    fn finish_fields(mut self, outcome: String, failed: bool, detail: &str) {
        self.finished = true;
        self.log.record(
            &format!("{}_end", self.event),
            if failed { "warn" } else { "info" },
            &format!(
                ",\"id\":{},\"operation\":\"{}\",\"elapsed_ms\":{}{}{detail}",
                self.id,
                self.operation,
                self.started.elapsed().as_millis(),
                outcome
            ),
        );
    }
}
impl Drop for Span<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.log.record(
                &format!("{}_abandoned", self.event),
                "error",
                &format!(
                    ",\"id\":{},\"operation\":\"{}\",\"unwinding\":{}{}",
                    self.id,
                    self.operation,
                    thread::panicking(),
                    self.start_detail
                ),
            );
        }
    }
}

fn epoch_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
fn optional_number(value: Option<i32>) -> String {
    value.map_or_else(|| "null".into(), |v| v.to_string())
}

fn workflow_failure(message:&str)->Option<&'static str>{
    // Never copy error text into diagnostics: only these fixed workflow codes.
    const CODES:&[&str]=&["remote_cleanup_stale","remote_cleanup_mapping","remote_cleanup_response","remote_branch_default","remote_branch_missing","remote_branch_changed","remote_branch_diagnostics","remote_branch_response","remote_branch_unconfirmed","saved_work_branch","saved_work_conflicts","saved_work_index","saved_work_limit","saved_work_upstream","saved_work_stale","saved_work_unknown","saved_work_changed","saved_work_paused","stash_rename_layout","stash_rename_limit","stash_rename_format","stash_rename_stale","lfs_prune_worktrees","lfs_prune_dirty","lfs_prune_layout","lfs_prune_verification","lfs_prune_response","lfs_prune_stale","lfs_prune_limit","lfs_prune_changed","lfs_prune_corrupt","lfs_prune_hash","lfs_prune_remote","lfs_prune_remote_missing","lfs_prune_remote_corrupt","browser_sign_in_failed","browser_sign_in_response"];
    const PATCH_SERIES_CODES: &[&str] = &[
        "patch_series_output_exists", "patch_series_destination", "patch_series_empty",
        "patch_series_limit", "patch_series_nonlinear", "patch_series_metadata",
        "patch_series_format", "patch_series_changed", "patch_series_output_changed",
        "patch_series_first_commit", "patch_series_unknown", "series_journal", "series_limit", "series_empty", "series_name", "series_order",
        "series_journal_changed", "series_state", "series_active", "series_dirty",
        "series_unresolved", "series_index", "series_format", "series_binary",
        "series_branch", "series_action", "series_state_changed", "series_repository",
        "series_stale", "series_branch_moved", "series_source_changed",
        "series_original_changed", "series_head_changed", "series_reference", "series_link",
    ];
    let(code,_)=message.split_once(':')?;
    const INTEGRATION_CODES: &[&str] = &[
        "integration_origin", "integration_auth", "integration_worker", "integration_worker_failed",
        "integration_worker_response", "integration_permission", "integration_fixture", "integration_request",
        "integration_transport", "integration_outcome_unknown", "integration_tls", "integration_network",
        "integration_response", "integration_unauthorized", "integration_forbidden", "integration_not_found",
        "integration_conflict", "integration_rejected", "integration_rate_limited", "integration_redirect",
        "integration_server", "integration_response_limit", "integration_json", "integration_request_limit",
        "integration_state_limit", "integration_lua_syntax", "integration_lua_memory", "integration_lua_limit",
        "integration_lua_result", "integration_lua_error", "integration_store", "integration_store_changed",
        "integration_store_limit", "integration_store_busy", "integration_files", "integration_files_limit", "integration_source",
        "integration_manifest", "integration_changed", "integration_folder", "integration_folder_limit",
        "integration_duplicate", "integration_example", "integration_exists", "integration_schema",
        "integration_connection", "integration_disabled", "integration_recovery", "integration_ui", "integration_ui_limit",
        "integration_profile", "integration_draft", "integration_draft_limit", "integration_draft_changed", "integration_draft_cancelled",
        "integration_stale", "integration_check", "integration_repository_changed",
        "playground_source", "playground_context", "playground_input", "playground_result",
        "playground_state", "playground_store", "playground_changed", "playground_busy",
        "ai_conflict_unsupported", "ai_conflict_input", "ai_conflict_limit", "ai_conflict_stale",
        "ai_proposal_limit", "ai_proposal_schema", "ai_proposal_sections", "ai_proposal_invalid",
        "ai_exchange_clock", "ai_exchange_files", "ai_exchange_link", "ai_exchange_profile",
        "ai_exchange_limit", "ai_exchange_exists", "ai_exchange_id", "ai_exchange_closed",
        "ai_exchange_missing", "ai_exchange_data", "ai_exchange_expired", "ai_exchange_cleanup",
    ];
    const COMPOSED_CODES: &[&str] = &[
        "workflow_review_stale", "commit_push_remote", "commit_push_branch", "commit_push_conflicts",
        "commit_push_empty", "commit_push_limit", "commit_push_stale", "commit_push_unconfirmed",
        "commit_push_commit_changed", "commit_push_destination_changed", "commit_push_incomplete",
        "saved_work_same_branch", "saved_work_switch_changed",
    ];
    CODES.iter().chain(PATCH_SERIES_CODES).chain(INTEGRATION_CODES).chain(COMPOSED_CODES).copied().find(|known|*known==code)
}
fn outcome<T>(result: &Result<T>) -> String {
    let (kind, code, os_error) = match result {
        Ok(_) => ("ok", None, None),
        Err(Error::Io { source, .. }) => ("io_error", None, source.raw_os_error()),
        Err(Error::Git { code, .. }) => ("git_error", *code, None),
        Err(Error::InvalidInput(_)) => ("invalid_input", None, None),
        Err(Error::Ssh(_)) => ("ssh_refused", None, None),
        Err(Error::InvalidOutput { .. }) => ("invalid_output", None, None),
        Err(Error::Unsupported(_)) => ("unsupported", None, None),
        Err(Error::SecretProtection(failure)) => (failure.code(), None, None),
        Err(Error::ExternalTool(failure)) => (
            "external_tool_failed",
            if let crate::ExternalToolFailure::Exit(code) = failure {
                *code
            } else {
                None
            },
            None,
        ),
        Err(Error::Ignore(_)) => ("ignore_refused", None, None),
        Err(Error::AtomicPush(_)) => ("atomic_push_refused", None, None),
        Err(Error::ForcePush(_)) => ("force_push_refused", None, None),
        Err(Error::Pull(_)) => ("pull_refused", None, None),
        Err(Error::Clone(_)) => ("clone_refused", None, None),
        Err(Error::CommitAction(_)) => ("commit_refused", None, None),
        Err(Error::Rebase(_)) => ("rebase_refused", None, None),
        Err(Error::Merge(_)) => ("merge_refused", None, None),
        Err(Error::Bisect(_)) => ("bisect_refused", None, None),
        Err(Error::Recovery(_)) => ("recovery_refused", None, None),
        Err(Error::Tag(_)) => ("tag_refused", None, None),
        Err(Error::Signing(_)) => ("signing_refused", None, None),
        Err(Error::Worktree(_)) => ("worktree_refused", None, None),
        Err(Error::Submodule(_)) => ("submodule_refused", None, None),
        Err(Error::Lfs(_)) => ("lfs_refused", None, None),
        Err(Error::Conflict(_)) => ("conflict_refused", None, None),
        Err(Error::Hosting(_)) => ("hosting_refused", None, None),
        Err(Error::Forge(failure)) => (failure.code(), None, None),
        Err(Error::FileEdit(_)) => ("file_edit_refused", None, None),
        Err(Error::Patch(_) | Error::PatchTarget { .. }) => ("patch_refused", None, None),
        Err(Error::GitFlow(_)) => ("gitflow_refused", None, None),
        // Only the fixed code: Claude Code's own message is never logged.
        Err(Error::CommitDraft { .. }) => ("commit_draft_refused", None, None),
        Err(Error::BranchReview(_)) => ("branch_review_refused", None, None),
        Err(Error::RepositoryBusy) => ("repository_busy", None, None),
        Err(Error::WorktreeRequired) => ("worktree_required", None, None),
        Err(Error::Cancelled) => ("cancelled", None, None),
        Err(Error::TimedOut) => ("timed_out", None, None),
        Err(Error::OutputLimit { .. }) => ("output_limit", None, None),
        Err(Error::WorkerPanicked) => ("worker_panicked", None, None),
        Err(Error::StatePoisoned(_)) => ("state_poisoned", None, None),
    };
    let mut detail = format!(",\"outcome\":\"{kind}\"");
    if let Err(Error::InvalidInput(message)) = result && let Some(code) = crate::undo::failure_code(message) {
        detail.push_str(&format!(",\"failure_code\":\"{code}\""));
    }
    if let Err(Error::InvalidInput(message))=result && let Some(code)=workflow_failure(message){detail.push_str(&format!(",\"failure_code\":\"{code}\""));}
    if let Err(error) = result
        && let Some(failure) = error.signing_failure()
    {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Forge(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Lfs(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(error) = result
        && let Some(failure) = error.lfs_lock_failure()
    {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Submodule(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Worktree(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Ignore(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::AtomicPush(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::ForcePush(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Tag(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::CommitAction(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Rebase(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Merge(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Bisect(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Recovery(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::FileEdit(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Patch(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::GitFlow(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::BranchReview(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::CommitDraft { failure, .. }) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::PatchTarget { failure, .. }) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Conflict(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::Hosting(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(Error::ExternalTool(failure)) = result {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Some(code) = code {
        detail.push_str(&format!(",\"exit_code\":{code}"));
    }
    if let Some(code) = os_error {
        detail.push_str(&format!(",\"os_error\":{code}"));
    }
    if let Err(error) = result
        && let Some(failure) = error.lfs_push_failure()
    {
        detail.push_str(&format!(
            ",\"failure_code\":\"{}\",\"classification\":\"diagnostic_hint\"",
            failure.code()
        ));
    }
    if let Err(error) = result
        && let Some(failure) = error.fetch_failure()
    {
        detail.push_str(&format!(
            ",\"failure_code\":\"{}\",\"classification\":\"diagnostic_hint\"",
            failure.code()
        ));
    }
    if let Err(error) = result
        && let Some(failure) = error.push_failure()
    {
        detail.push_str(&format!(
            ",\"failure_code\":\"{}\",\"classification\":\"diagnostic_hint\"",
            failure.code()
        ));
    }
    if let Err(error) = result
        && let Some(failure) = error.pull_failure()
    {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
    }
    if let Err(error) = result
        && let Some(failure) = error.clone_failure()
    {
        detail.push_str(&format!(",\"failure_code\":\"{}\"", failure.code()));
        if matches!(
            failure,
            crate::CloneFailure::Transport(_) | crate::CloneFailure::TransportNotAllowed
        ) {
            detail.push_str(",\"classification\":\"diagnostic_hint\"");
        }
    }
    if let Err(Error::Ssh(failure)) = result { detail.push_str(&format!(",\"failure_code\":\"{}\"",failure.code())); }
    if let Err(Error::OutputLimit { stream, limit }) = result {
        let stream = if *stream == "stdout" {
            "stdout"
        } else if *stream == "stderr" {
            "stderr"
        } else {
            "other"
        };
        detail.push_str(&format!(",\"stream\":\"{stream}\",\"limit\":{limit}"));
    }
    detail
}

fn operation_label(value: &str) -> &'static str {
    const OPERATIONS: &[&str] = &[
        "crash-report-read", "crash-report-export",
        "managed-git-verify", "git-selection-save",
        "benchmark", "benchmark-prepare", "benchmark-refresh", "benchmark-history", "benchmark-diff",
        "info",
        "status",
        "auto_refresh",
        "watch",
        "library-check",
        "library-scan",
        "library-scan-add",
        "repository-summary",
        "fetch-all",
        "pull-all",
        "load_state",
        "save_state",
        "terminal_start",
        "terminal_ready",
        "terminal_input_queued",
        "terminal_interrupt",
        "terminal_clear_scrollback",
        "terminal_stop",
        "terminal_exit",
        "agent-session-preview", "agent-session-start", "agent-session-remove",
        "agent-session-changes", "agent-session-diff",
        "agent-integration-preview", "agent_integration_reviewed", "agent_attention",
        "agent_conversation_ready", "agent_turn_working", "agent_waiting_input",
        "agent_turn_finished", "agent_activity_changed", "agent_activity_unavailable",
        "agent_start_requested", "agent_setup_started", "agent_setup_succeeded",
        "agent_started", "agent_start_failed", "agent_stop_requested",
        "agent_stopped", "agent_exited", "agent_worktree_removed",
        "file-preview",
        "conflict-preview",
        "conflict-show",
        "conflict-edit",
        "conflict-save",
        "conflict-save-cancel",
        "conflict-keep-review",
        "conflict-keep",
        "conflict-keep-cancel",
        "hosting-connect",
        "hosting-account",
        "hosting-create-preview",
        "integration-run", "integration-read", "integration-submit",
        "integration-discover", "integration-save", "integration-review",
        "integration-draft-save", "integration-profile-save",
        "integration-package",
        "integration-validate",
        "playground-run", "playground-load", "playground-save",
        "ai-conflict-prepare", "ai-conflict-share", "ai-conflict-apply", "ai-conflict-claude",
        "ai-conflict-codex",
        "integration-validation-result",
        "integration-submit-result",
        "hosting-list",
        "hosting-search",
        "commit-push-preview",
        "commit-push",
        "saved-switch",
        "hosting-preview",
        "hosting-create",
        "hosting-pull-details",
        "hosting-review-preview",
        "hosting-review-submit",
        "hosting-merge-preview",
        "hosting-merge-submit",
        "hosting-thread-details",
        "hosting-thread-preview",
        "hosting-thread-submit",
        "hosting-drafts-load",
        "hosting-drafts-save",
        "hosting-ci-marks",
        "hosting-ci-details",
        "hosting-ci-rerun-preview",
        "hosting-ci-rerun",
        "hosting-ci-runs",
        "hosting-ci-run",
        "github-publish-preview",
        "github-publish",
        "hosting-accounts",
        "ssh-list", "ssh-discover", "ssh-create", "ssh-unlock", "ssh-unload", "ssh-check",
        "ssh-binding-review", "ssh-binding-save", "ssh-prompt", "ssh-stop", "ssh-remember", "ssh-forget",
        "github-sign-in",
        "github-repositories",
        "github-notifications",
        "github-repository",
        "github-pulls",
        "github-open",
        "github-search",
        "preview-open",
        "preview-sign-in",
        "keep-preview",
        "mark-resolved",
        "stage-selection",
        "unstage-selection",
        "head",
        "branches",
        "remotes",
        "remote-add",
        "remote-edit",
        "remote-remove",
        "remote-check",
        "branch-merge-state",
        "push-outlook",
        "init",
        "identity-read",
        "identity-save",
        "issue-links-read",
        "issue-links-save",
        "issue-link-open",
        "custom-url-open",
        "gitflow-read",
        "gitflow-init-preview",
        "gitflow-init",
        "gitflow-start-preview",
        "gitflow-start",
        "gitflow-finish-preview",
        "gitflow-finish",
        "commit-draft",
        "branch-review-preview", "branch-review", "branch-review-cancel",
        "review-merge-base", "review-diff", "review-check-refs",
        "commit-activity",
        "activity-treemap",
        "history-search",
        "file-search",
        "file-search-open",
        "review-window",
        "review-close",
        "commit-link",
        "commit-link-open",
        "app-update-check",
        "app-update-download",
        "app-update-prepare",
        "app-update-install",
        "commit-url",
        "review",
        "commit-template",
        "auto-fetch",
        "custom-command",
        "file-history",
        "directory-history",
        "file-content",
        "file-backups",
        "file-discard",
        "file-restore",
        "file-recover",
        "diff-search",
        "patch-preview",
        "patch-export-preview",
        "patch-import",
        "patch-export",
        "discard-preview",
        "discard-selection",
        "tree-list",
        "tree-content",
        "map-code", "map-history", "map-people", "map-export",
        "compare",
        "compare-base",
        "compare-target",
        "compare-files",
        "compare-check",
        "submodule-diff",
        "read-submodule-history",
        "read-local-commits",
        "verify-lfs-object",
        "read-local-snapshot",
        "compare-diff",
        "compare-cancel",
        "file-blame",
        "file-diff",
        "fetch",
        "pull",
        "pull-fetch",
        "pull-preview",
        "pull-apply",
        "push",
        "ignore-read",
        "ignore-preview",
        "ignore-save",
        "ignore-tracked-preview", "ignore-tracked", "stash-snapshot-preview", "stash-snapshot",
        "patch-series-preview", "patch-series-export", "format-patch",
        "workflow-preview",
        "remote-cleanup-preview",
        "remote-prune",
        "remote-branch-delete-preview",
        "remote-branch-delete",
        "saved-work-preview",
        "saved-work",
        "stash-rename-preview",
        "stash-rename",
        "stash-control-preview", "stash-control", "stash-selection-preview", "stash-selection",
        "stash-apply-preview", "stash-apply-selection", "stash-show", "stash-pop",
        "remote-rename-preview", "remote-rename", "branch-fast-forward-preview", "branch-fast-forward",
        "submodule-batch-preview", "submodule-batch", "submodule-deinitialize",
        "lfs-prune-preview",
        "lfs-prune",
        "hosting-browser-sign-in",
        "atomic-push-preview",
        "atomic-push",
        "force-push-preview",
        "force-push-policy",
        "atomic-push-policy",
        "remote-tag-policy",
        "pull-policy",
        "mcp-input-validation",
        "force-push",
        "upstream",
        "history",
        "diff",
        "diff-words",
        "image-preview",
        "image-difference",
        "external-editor",
        "external-diff",
        "external-merge",
        "external-prepare",
        "external-process",
        "external-result",
        "external-import",
        "external-settings",
        "external-cancel",
        "image-decode-before",
        "image-decode-after",
        "staged",
        "hunks",
        "stage",
        "unstage",
        "stage-all",
        "unstage-all",
        "commit",
        "commit-preview",
        "undo-preview",
        "undo-commit",
        "commit-prediction",
        "rebase-prediction",
        "rebase-preview",
        "merge-preview",
        "bisect",
        "bisect-status",
        "bisect-auto",
        "bisect-auto-preview",
        "bisect-tests",
        "bisect-preview",
        "bisect-start",
        "bisect-good",
        "bisect-bad",
        "bisect-skip",
        "bisect-resume",
        "bisect-restore",
        "recovery-preview",
        "lost-commits",
        "lost-recovery-preview",
        "tags",
        "tag-inspect",
        "signing-read",
        "signing-save",
        "signature-verify",
        "tag-sign",
        "tag-preview",
        "tag-create",
        "tag-delete",
        "tag-delete-batch",
        "remote-tags",
        "remote-tag-preview",
        "remote-tag-publish",
        "remote-tag-delete",
        "remote-tag-publish-batch",
        "worktrees",
        "submodules",
        "lfs-status",
        "lfs-locks",
        "lfs-lock-preview",
        "lfs-lock",
        "lfs-unlock",
        "lfs-force-unlock",
        "lfs-preview",
        "lfs-fetch",
        "lfs-push",
        "lfs-checkout",
        "lfs-track",
        "lfs-untrack",
        "lfs-version",
        "lfs-hash",
        "submodule-preview",
        "submodule-open",
        "submodule-update",
        "submodule-init",
        "submodule-fetch",
        "submodule-checkout",
        "submodule-retain",
        "submodule-source",
        "submodule-add",
        "submodule-remove",
        "submodule-sync",
        "submodule-set-url",
        "submodule-move",
        "branch-sync",
        "branch-finish",
        "custom-command-import",
        "custom-command-export",
        "submodule-action-preview",
        "worktree-open",
        "worktree-preview",
        "worktree-create",
        "worktree-remove",
        "worktree-move",
        "worktree-repair",
        "worktree-prune",
        "reflog",
        "recover-branch",
        "reset-soft",
        "reset-mixed",
        "reset-hard",
        "rebase",
        "interactive-rebase",
        "series-preview", "series-import", "series-continue", "series-skip", "series-abort",
        "rebase-continue",
        "rebase-skip",
        "rebase-abort",
        "amend",
        "cherry-pick",
        "cherry-pick-continue",
        "cherry-pick-skip",
        "cherry-pick-abort",
        "revert",
        "revert-continue",
        "revert-skip",
        "revert-abort",
        "branch-create",
        "branch-switch",
        "branch-checkout-remote",
        "branch-rename",
        "branch-delete",
        "branch-delete-batch",
        "stashes",
        "operation",
        "stash-push",
        "stash-apply",
        "stash-drop",
        "stash-preview",
        "merge",
        "merge-continue",
        "merge-abort",
        "merge-finish",
        "open",
        "discover",
        "clone",
        "resolve-commit",
    ];
    OPERATIONS
        .iter()
        .copied()
        .find(|&known| known == value)
        .unwrap_or("other")
}

fn command_label(arguments: &[OsString]) -> &'static str {
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let Some(argument) = argument.to_str() else {
            return "other";
        };
        if matches!(argument, "-c" | "--git-dir" | "--work-tree") {
            arguments.next();
            continue;
        }
        if argument.starts_with("--config-env=")
            || matches!(
                argument,
                "--no-literal-pathspecs" | "--literal-pathspecs" | "--no-replace-objects"
            )
        {
            continue;
        }
        const COMMANDS: &[&str] = &[
            "init",
            "--version",
            "rev-parse",
            "symbolic-ref",
            "show-ref",
            "status",
            "for-each-ref",
            "rev-list",
            "log",
            "blame",
            "ls-tree",
            "ls-remote",
            "merge-base",
            "merge-tree",
            "check-attr",
            "cat-file",
            "grep",
            "diff",
            "diff-tree",
            "format-patch",
            "mailinfo", "am",
            "show",
            "check-ignore",
            "check-ref-format",
            "add",
            "apply",
            "update-index",
            "credential",
            "curl", "lua-worker",
            "claude",
            "codex",
            "ssh-add", "ssh-keygen", "ssh-config",
            "ls-files",
            "rm",
            "restore",
            "commit",
            "cherry-pick",
            "revert",
            "rebase",
            "reflog",
            "fsck",
            "reset",
            "update-ref",
            "mktag",
            "tag",
            "verify-tag",
            "verify-commit",
            "worktree",
            "submodule",
            "lfs",
            "locks",
            "lock",
            "unlock",
            "checkout",
            "hash-object",
            "var",
            "branch",
            "config",
            "fetch",
            "clone",
            "push",
            "remote",
            "switch",
            "stash",
            "merge",
        ];
        return COMMANDS
            .iter()
            .copied()
            .find(|&known| known == argument)
            .unwrap_or("other");
    }
    "other"
}

fn path(directory: &Path, index: usize) -> PathBuf {
    directory.join(if index == 0 {
        "rocket.jsonl".into()
    } else {
        format!("rocket.{index}.jsonl")
    })
}

fn regular_or_missing(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log path must be a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn append(directory: &Path, record: &[u8], max_bytes: u64, wait_on_contention: bool) -> io::Result<bool> {
    if record.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "diagnostic record exceeds file limit",
        ));
    }
    let lock_path = directory.join("rocket.lock");
    regular_or_missing(&lock_path)?;
    // A fresh handle per append prevents recursive locking via cloned handles.
    // Drop releases the OS lock even on an error or process crash; no stale PID lock.
    let lock = options().open(lock_path)?;
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if wait_on_contention && started.elapsed() < LOCK_WAIT => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(TryLockError::WouldBlock) => {
                overflow::save(directory, record)?;
                return Ok(true);
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
    overflow::drain(directory, max_bytes)?;
    append_record(directory, record, max_bytes)?;
    Ok(false)
}

// The caller holds the rotation lock. Shared by normal writes and crash-safe
// overflow recovery; replay can duplicate a sequence but cannot lose the source.
fn append_record(directory: &Path, record: &[u8], max_bytes: u64) -> io::Result<()> {
    // Validate all rotation targets before changing any. No globbing or recursive deletion.
    for index in 0..FILES {
        regular_or_missing(&path(directory, index))?;
    }
    let current = path(directory, 0);
    let mut file = options().open(&current)?;
    let length = file.metadata()?.len();
    // A killed process or failed write may leave a partial line. Archive it and
    // start a fresh file so subsequent JSON records remain independently readable.
    let incomplete = if length > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        last[0] != b'\n'
    } else {
        false
    };
    if incomplete || length.saturating_add(record.len() as u64) > max_bytes {
        drop(file);
        // Oversized preexisting files are discarded, not moved beyond the cap.
        for index in (0..FILES).rev() {
            let source = path(directory, index);
            match fs::metadata(&source) {
                Ok(metadata) if index == FILES - 1 || metadata.len() > max_bytes => {
                    fs::remove_file(&source)?
                }
                Ok(_) => fs::rename(&source, path(directory, index + 1))?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        file = options().open(current)?;
    }
    file.seek(SeekFrom::End(0))?;
    file.write_all(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        process::{Command, Stdio},
    };
    #[test]
    fn agent_operations_keep_lifecycle_labels_but_never_payloads() {
        let directory=Directory::new();
        let log=directory.log();
        let operations=["agent-session-changes","agent-session-diff","agent-session-preview","agent-session-start","agent-session-remove","agent_start_requested","agent_setup_started","agent_setup_succeeded","agent_started","agent_start_failed","agent_stop_requested","agent_stopped","agent_exited","agent_worktree_removed"];
        for operation in operations {
            let result:Result<()>=log.request(operation,|_|Err(Error::InvalidInput("PRIVATE_PROGRAM PRIVATE_ARGUMENT PRIVATE_OUTPUT".into())));
            assert!(result.is_err());
        }
        let _:Result<()>=log.request("agent_PRIVATE_TASK",|_|Err(Error::Cancelled));
        let text=directory.current();
        for operation in operations {assert_eq!(text.matches(&format!("\"operation\":\"{operation}\"")).count(),2);}
        assert!(!text.contains("PRIVATE_"));
        assert!(text.contains("\"operation\":\"other\""));
        assert!(text.contains("\"outcome\":\"cancelled\""));
        assert_records(&text);
    }
    #[test]
    fn health_read_does_not_wait_for_the_writer_io_lock() {
        let directory = Directory::new();
        let log = directory.log();
        let writer = log.inner.state.lock().unwrap();
        let reader = log.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || send.send(reader.health()).unwrap());
        // Hold the actual writer lock until after the reader's deadline. This
        // detects the blocking dependency without timing filesystem operations.
        let health = receive.recv_timeout(Duration::from_secs(1));
        drop(writer);
        worker.join().unwrap();
        assert!(health.is_ok(), "status-bar health reads waited for log I/O");
        assert_eq!(health.unwrap().dropped_events, 0);
    }
    #[test]
    fn integration_store_busy_keeps_only_the_fixed_diagnostic_code() {
        let detail = outcome::<()>(&Err(Error::InvalidInput(
            "integration_store_busy: PRIVATE_PATH PRIVATE_TOKEN".into(),
        )));
        assert!(detail.contains("\"failure_code\":\"integration_store_busy\""));
        assert!(!detail.contains("PRIVATE_"));
    }
    #[test]
    fn undo_codes_never_copy_commit_or_path_payloads() {
        let detail = outcome::<()>(&Err(Error::InvalidInput("[undo_stale_preview] PRIVATE_PATH message".into())));
        assert!(detail.contains("\"failure_code\":\"undo_stale_preview\""));
        assert!(!detail.contains("PRIVATE_PATH"));
        assert!(crate::undo::failure_code("[injected] private").is_none());
        assert!(crate::undo::failure_code("[undo_stale_preview\"bad] private").is_none());
        let directory = Directory::new();
        let log = directory.log();
        for operation in ["undo-preview", "undo-commit"] {
            let result: Result<()> = log.request(operation, |_| {
                Err(Error::InvalidInput("[undo_stale_preview] PRIVATE_PATH message".into()))
            });
            assert!(result.is_err());
        }
        let text = directory.current();
        for operation in ["undo-preview", "undo-commit"] {
            assert_eq!(text.matches(&format!("\"operation\":\"{operation}\"")).count(), 2);
        }
        assert!(!text.contains("PRIVATE_PATH"));
        assert_eq!(text.matches("\"failure_code\":\"undo_stale_preview\"").count(), 2);
        assert_records(&text);
    }
    #[test]
    fn workflow_codes_are_fixed_and_never_copy_error_payloads(){
        let detail=outcome::<()>(&Err(Error::InvalidInput("saved_work_stale: PRIVATE_PATH secret".into())));
        assert!(detail.contains("\"failure_code\":\"saved_work_stale\""));assert!(!detail.contains("PRIVATE"));assert!(!detail.contains("secret"));
        assert!(workflow_failure("injected_code: private").is_none());assert!(workflow_failure("saved_work_stale\"bad: private").is_none());
        for code in ["playground_changed","playground_source","ai_conflict_stale","ai_exchange_expired","ai_proposal_schema"] {
            let detail=outcome::<()>(&Err(Error::InvalidInput(format!("{code}: PRIVATE_SOURCE PRIVATE_PROPOSAL"))));
            assert!(detail.contains(&format!("\"failure_code\":\"{code}\"")));assert!(!detail.contains("PRIVATE_"));
        }
        for code in ["workflow_review_stale", "commit_push_stale", "commit_push_incomplete", "saved_work_switch_changed"] {
            let detail=outcome::<()>(&Err(Error::InvalidInput(format!("{code}: PRIVATE_MESSAGE PRIVATE_DESTINATION"))));
            assert!(detail.contains(&format!("\"failure_code\":\"{code}\"")));
            assert!(!detail.contains("PRIVATE_"));
        }
    }

    #[test]
    fn patch_series_errors_keep_only_known_codes() {
        for code in ["series_stale","series_source_changed","series_state_changed"] {
            let detail=outcome::<()>(&Err(Error::InvalidInput(format!("{code}: PRIVATE_PATCH_CONTENT"))));
            assert!(detail.contains(&format!("\"failure_code\":\"{code}\"")));
            assert!(!detail.contains("PRIVATE"));
        }
        let detail = outcome::<()>(&Err(Error::InvalidInput(
            "patch_series_output_changed: PRIVATE_PATH secret patch bytes".into(),
        )));
        assert!(detail.contains("\"failure_code\":\"patch_series_output_changed\""));
        assert!(!detail.contains("PRIVATE_PATH"));
        assert!(!detail.contains("secret"));
        assert!(!detail.contains("patch bytes"));
        let untrusted = outcome::<()>(&Err(Error::InvalidInput(
            "patch_series_output_changed\"injected: PRIVATE_PAYLOAD".into(),
        )));
        assert!(!untrusted.contains("failure_code"));
        assert!(!untrusted.contains("PRIVATE"));
    }

    #[test]
    fn patch_and_submodule_outcomes_separate_operation_from_severity() {
        let directory = Directory::new();
        let log = directory.log();
        log.patch_outcome("patch-import", 2, 2, 2, true, true);
        log.patch_outcome("patch-import", 2, 1, 1, false, false);
        log.submodule_action_outcome("submodule-move", &["checkout_moved"], true, true);
        log.submodule_action_outcome("submodule-move", &[], true, false);
        let text = directory.current();
        assert_records(&text);
        for (event, operation) in [
            ("patch_outcome", "patch-import"),
            ("submodule_action_outcome", "submodule-move"),
        ] {
            let records: Vec<_> = text.lines().filter(|line| line.contains(&format!("\"event\":\"{event}\""))).collect();
            assert_eq!(records.len(), 2);
            assert!(records.iter().all(|line| line.contains(&format!("\"operation\":\"{operation}\""))));
            assert!(records[0].contains("\"level\":\"info\""));
            assert!(records[1].contains("\"level\":\"warn\""));
        }
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn github_publish_parse_errors_are_visible_without_private_output() {
        let directory = Directory::new();
        let log = directory.log();
        let report = crate::GitHubPublishReport {
            creation: crate::GitHubCreation::Created,
            remote_added: true,
            push: Some(crate::PushReport {
                result: Ok(crate::WriteOutput { stdout: b"PRIVATE git output".to_vec(), stderr: vec![] }),
                update: Some(Err(Error::InvalidOutput { operation: "push", detail: "PRIVATE parse detail".into() })),
                remote_outcome: crate::PushRemoteOutcome::Unknown,
                error_diagnostics: false,
                upstream: None,
                status: None,
                branches: Ok(vec![]),
            }),
            error: None,
            url: "https://github.com/PRIVATE/repository".into(),
        };
        log.github_publish_outcome(&report);
        let text = directory.current();
        assert_records(&text);
        let record = text.lines().find(|line| line.contains("\"event\":\"github_publish_outcome\"")).unwrap();
        assert!(record.contains("\"level\":\"warn\""));
        assert!(record.contains("\"has_errors\":true"));
        assert!(record.contains("\"remote_outcome\":\"unknown\""));
        assert!(!text.contains("PRIVATE"));
        assert_eq!(log.health().dropped_events, 0);
    }

    pub(super) struct Directory(pub(super) PathBuf);
    impl Directory {
        pub(super) fn new() -> Self {
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/log-tests");
            fs::create_dir_all(&base).unwrap();
            let path = base.join(format!(
                "{}-{}-{}",
                std::process::id(),
                epoch_nanos(),
                NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn log(&self) -> DiagnosticLog {
            DiagnosticLog::open(&self.0).unwrap()
        }
        fn current(&self) -> String {
            fs::read_to_string(path(&self.0, 0)).unwrap()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../work/log-tests")
                .canonicalize()
                .unwrap();
            let target = self.0.canonicalize().unwrap();
            assert_eq!(target.parent(), Some(base.as_path()));
            fs::remove_dir_all(target).unwrap();
        }
    }

    fn assert_records(text: &str) {
        assert!(text.ends_with('\n'));
        for line in text.lines() {
            assert!(
                line.starts_with("{\"schema\":1,") && line.ends_with('}'),
                "{line}"
            );
            assert_eq!(
                line.matches("\"event\":").count(),
                1,
                "interleaved record: {line}"
            );
            assert!(!line.contains('\r'));
        }
    }

    #[test]
    fn mcp_request_timing_excludes_unknown_labels_and_payloads() {
        let directory = Directory::new();
        let log = directory.log();
        let mut metrics = McpRequestMetrics {
            native_request: 8,
            tool: "rocket_click",
            phase: "queued",
            outcome: "refused",
            elapsed_ms: 21000,
            queue_ms: 21000,
            validation_ms: 0,
            phase_ms: 21000,
            input_delivered: false,
        };
        log.mcp_request(&metrics);
        metrics.tool = "PRIVATE_TOKEN";
        metrics.phase = "PRIVATE_PATH";
        metrics.outcome = "PRIVATE_ERROR";
        log.mcp_request(&metrics);
        let text = directory.current();
        assert_records(&text);
        assert!(text.contains("\"event\":\"mcp_request\""));
        assert!(text.contains("\"queue_ms\":21000"));
        assert!(text.contains("\"input_delivered\":false"));
        assert!(!text.contains("PRIVATE_"));
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn rotation_bounds_files_and_preserves_unrelated_data() {
        let directory = Directory::new();
        fs::write(directory.0.join("keep.log"), "unrelated").unwrap();
        let log = DiagnosticLog::open_with_limit(&directory.0, 2048).unwrap();
        for _ in 0..50 {
            log.request("status", |_| Ok(())).unwrap();
        }
        for index in 0..FILES {
            let file = path(&directory.0, index);
            assert!(fs::metadata(&file).unwrap().len() <= 2048);
            assert_records(&fs::read_to_string(file).unwrap());
        }
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), FILES + 2);
        assert_eq!(
            fs::read(directory.0.join("keep.log")).unwrap(),
            b"unrelated"
        );
        assert!(directory.current().contains("\"seq\":101,"));
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn independent_loggers_and_threads_keep_all_records_whole() {
        let directory = Directory::new();
        let logs: Vec<_> = (0..4).map(|_| directory.log()).collect();
        thread::scope(|scope| {
            for log in &logs {
                // Share each logger as well as the directory across independent handles.
                for _ in 0..2 {
                    scope.spawn(move || {
                        for _ in 0..20 {
                            log.request("status", |_| Ok(())).unwrap();
                        }
                    });
                }
            }
        });
        for log in &logs {
            assert_eq!(log.health().dropped_events, 0);
        }
        let text = directory.current();
        assert_records(&text);
        assert_eq!(text.lines().count(), 4 + 8 * 20 * 2);
        assert_eq!(
            logs.iter()
                .map(|log| log.session_id())
                .collect::<HashSet<_>>()
                .len(),
            4
        );
        for log in &logs {
            let lines: Vec<_> = text
                .lines()
                .filter(|line| line.contains(log.session_id()))
                .collect();
            assert_eq!(lines.len(), 81);
            for (index, line) in lines.iter().enumerate() {
                assert!(line.contains(&format!("\"seq\":{},", index + 1)));
            }
        }
    }

    #[test]
    fn busy_lock_saves_durable_overflow_and_recovers_without_drops() {
        let directory = Directory::new();
        let log = directory.log();
        let lock = options().open(directory.0.join("rocket.lock")).unwrap();
        lock.try_lock().unwrap();
        let start = Instant::now();
        assert_eq!(log.request("status", |_| Ok(42)).unwrap(), 42);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(log.health().dropped_events, 0);
        assert_eq!(log.health().overflow_events, 2);
        let saved: String = DiagnosticLog::retained_files(&directory.0).unwrap().iter()
            .filter_map(|p| fs::read_to_string(p).ok()).collect();
        assert_eq!(saved.lines().count(), 3);
        assert_records(&saved);
        drop(lock);
        log.request("status", |_| Ok(())).unwrap();
        assert_eq!(directory.current().lines().count(), 5);
        assert_eq!(fs::read_dir(directory.0.join("rocket-overflow")).unwrap().count(), 0);
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn known_contention_recovers_without_repeated_lock_waits() {
        let directory = Directory::new();
        let log = directory.log();
        let lock = options().open(directory.0.join("rocket.lock")).unwrap();
        lock.try_lock().unwrap();
        let start = Instant::now();
        for value in 0..8 {
            assert_eq!(log.request("status", |_| Ok(value)).unwrap(), value);
        }
        let elapsed = start.elapsed();
        assert_eq!(log.health().overflow_events, 16);
        assert_eq!(log.health().dropped_events, 0);
        drop(lock);
        log.request("status", |_| Ok(())).unwrap();
        assert_eq!(directory.current().lines().count(), 19);
        assert_records(&directory.current());
        assert_eq!(fs::read_dir(directory.0.join("rocket-overflow")).unwrap().count(), 0);
        // The original request budget also covers a burst once contention is
        // known; later events must not each spend another full lock wait.
        eprintln!("contended request burst: {elapsed:?} (budget: 3 seconds)");
        assert!(elapsed < Duration::from_secs(3), "contended request burst took {elapsed:?}");
    }

    #[test]
    fn invalid_rotation_target_preserves_files_and_exposes_logging_failure() {
        let directory = Directory::new();
        let log = directory.log();
        let before = directory.current();
        fs::create_dir(path(&directory.0, 3)).unwrap();
        let result: Result<()> = log.request("commit", |_| {
            Err(Error::InvalidInput("original failure".into()))
        });
        assert!(
            matches!(result, Err(Error::InvalidInput(message)) if message == "original failure")
        );
        assert_eq!(directory.current(), before);
        assert_eq!(log.health().dropped_events, 2);
        assert!(DiagnosticLog::open(&directory.0).is_err());
        fs::remove_dir(path(&directory.0, 3)).unwrap();
        log.request("status", |_| Ok(())).unwrap();
        assert!(directory.current().contains("\"event\":\"request_end\""));
        assert!(directory.current().contains("\"dropped_before\":2,"));
    }

    #[test]
    fn brief_contention_keeps_records_when_another_writer_is_descheduled() {
        let directory = Directory::new();
        let log = directory.log();
        let lock = options().open(directory.0.join("rocket.lock")).unwrap();
        lock.try_lock().unwrap();
        thread::scope(|scope| {
            let (started, ready) = std::sync::mpsc::channel();
            let log = &log;
            let worker = scope.spawn(move || {
                started.send(()).unwrap();
                log.request("status", |_| Ok(())).unwrap();
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            // Model a writer preempted while rotating, without any Git/network work.
            thread::sleep(Duration::from_millis(200));
            drop(lock);
            worker.join().unwrap();
        });
        assert_eq!(log.health().dropped_events, 0, "{:?}", log.health());
        assert_eq!(directory.current().lines().count(), 3);
    }

    #[test]
    fn untrusted_strings_and_error_payloads_never_enter_diagnostics() {
        let directory = Directory::new();
        let log = directory.log();
        let secret = concat!("PRIVATE", "-SECRET-\"\nC:/Users/name/token");
        let errors = [
            Error::Io {
                action: "PRIVATE action",
                source: io::Error::other(secret),
            },
            Error::Git {
                operation: "PRIVATE operation",
                code: Some(128),
                stdout: secret.as_bytes().to_vec(),
                stderr: secret.as_bytes().to_vec(),
            },
            Error::InvalidInput(secret.into()),
            Error::InvalidOutput {
                operation: "PRIVATE parser",
                detail: secret.into(),
            },
            Error::Unsupported(secret.into()),
            Error::RepositoryBusy,
            Error::WorktreeRequired,
            Error::Cancelled,
            Error::TimedOut,
            Error::OutputLimit {
                stream: "PRIVATE stream",
                limit: 128,
            },
            Error::WorkerPanicked,
            Error::StatePoisoned("PRIVATE state"),
        ];
        for error in errors {
            let result: Result<()> = log.request(secret, |_| Err(error));
            assert!(result.is_err());
        }
        let text = directory.current();
        assert!(!text.contains("PRIVATE"));
        assert!(text.contains("\"operation\":\"other\""));
        assert!(text.contains("\"outcome\":\"invalid_output\""));
        assert!(text.contains("\"exit_code\":128"));
        assert_records(&text);
    }

    #[test]
    fn stash_preview_logs_a_named_cancelled_request_without_payloads() {
        let directory = Directory::new();
        let log = directory.log();
        let result: Result<()> = log.request("stash-preview", |_| Err(Error::Cancelled));
        assert!(result.is_err());
        let text = directory.current();
        assert!(text.contains("\"operation\":\"stash-preview\""));
        assert!(text.contains("\"outcome\":\"cancelled\""));
        assert_records(&text);
    }

    #[test]
    fn panicking_request_is_marked_without_logging_the_payload() {
        let directory = Directory::new();
        let log = directory.log();
        let result = std::panic::catch_unwind(|| {
            log.request::<()>("commit", |context| {
                context.step("commit", "write_result", || panic!("private panic message"))
            })
        });
        assert!(result.is_err());
        log.request("status", |_| Ok(())).unwrap();
        let text = directory.current();
        assert!(text.contains("\"event\":\"request_abandoned\""));
        assert!(
            text.lines()
                .any(|line| line.contains("\"event\":\"step_abandoned\"")
                    && line.contains("\"step\":\"write_result\""))
        );
        assert!(text.contains("\"unwinding\":true"));
        assert!(!text.contains("private panic"));
        assert_eq!(log.health().dropped_events, 0);
    }

    #[test]
    fn partial_tail_is_archived_and_new_records_start_on_a_clean_line() {
        let directory = Directory::new();
        let log = directory.log();
        OpenOptions::new()
            .append(true)
            .open(path(&directory.0, 0))
            .unwrap()
            .write_all(b"{partial")
            .unwrap();
        log.request("status", |_| Ok(())).unwrap();
        assert_records(&directory.current());
        assert!(
            fs::read_to_string(path(&directory.0, 1))
                .unwrap()
                .ends_with("{partial")
        );
        assert_eq!(log.health().dropped_events, 0);
    }

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn child(directory: &Directory, mode: &str) -> ChildGuard {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "diagnostics::tests::writer_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("ROCKET_LOG_TEST_DIR", &directory.0)
            .env("ROCKET_LOG_TEST_MODE", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        ChildGuard(command.spawn().unwrap())
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "log fixture did not start");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "subprocess fixture invoked by logging tests"]
    fn writer_process_fixture() {
        let directory = PathBuf::from(std::env::var_os("ROCKET_LOG_TEST_DIR").unwrap());
        let mode = std::env::var("ROCKET_LOG_TEST_MODE").unwrap();
        if mode == "hold" {
            let lock = options().open(directory.join("rocket.lock")).unwrap();
            lock.try_lock().unwrap();
            fs::write(directory.join("locked"), b"ready").unwrap();
            thread::sleep(Duration::from_secs(30));
        } else {
            let log = DiagnosticLog::open_with_limit(&directory, 4096).unwrap();
            fs::write(directory.join(format!("ready-{}", std::process::id())), b"").unwrap();
            wait_for(|| directory.join("go").is_file());
            for _ in 0..40 {
                log.request("status", |_| Ok(())).unwrap();
            }
            assert_eq!(log.health().dropped_events, 0, "{:?}", log.health());
        }
    }

    #[test]
    fn independent_processes_rotate_without_interleaving_or_exceeding_bounds() {
        let directory = Directory::new();
        let mut children: Vec<_> = (0..3).map(|_| child(&directory, "write")).collect();
        wait_for(|| {
            fs::read_dir(&directory.0)
                .unwrap()
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-"))
                .count()
                == 3
        });
        fs::write(directory.0.join("go"), b"").unwrap();
        for child in &mut children {
            assert!(child.0.wait().unwrap().success());
        }
        for index in 0..FILES {
            let file = path(&directory.0, index);
            assert!(fs::metadata(&file).unwrap().len() <= 4096);
            assert_records(&fs::read_to_string(file).unwrap());
        }
    }

    #[test]
    fn process_death_releases_the_log_lock_without_manual_cleanup() {
        let directory = Directory::new();
        let mut child = child(&directory, "hold");
        wait_for(|| directory.0.join("locked").is_file());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let log = directory.log();
        log.request("status", |_| Ok(())).unwrap();
        assert_eq!(log.health().dropped_events, 0);
        assert_records(&directory.current());
    }
}
