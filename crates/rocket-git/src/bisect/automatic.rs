//! Reviewed test commands drive the existing manual state machine. The command
//! runs with normal user permissions; it is contained for lifecycle, not sandboxed.
use super::*;
use std::{
    ffi::OsString,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_TESTS: usize = 256;
const OUTPUT_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BisectTestCommand {
    pub program: PathBuf,
    /// Literal arguments: no shell parsing or placeholder substitution.
    pub arguments: Vec<String>,
    pub timeout_seconds: u64,
}
impl BisectTestCommand {
    pub fn validate(&self) -> Result<()> {
        if !crate::valid_custom_program_path(&self.program) {
            return Err(Error::InvalidInput("Bisect test: choose a local absolute executable path (.exe on Windows). Select a shell explicitly to run a script.".into()));
        }
        if self.arguments.len() > 64
            || self.arguments.iter().map(|a| a.len() + 1).sum::<usize>() > 16000
            || self
                .arguments
                .iter()
                .any(|a| a.chars().any(crate::display_text::hidden))
        {
            return Err(Error::InvalidInput("Bisect test: use at most 64 literal arguments and 16,000 bytes, without hidden/control characters.".into()));
        }
        if !(1..=3600).contains(&self.timeout_seconds) {
            return Err(Error::InvalidInput(
                "Bisect test timeout must be 1 to 3,600 seconds per revision.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectTestOutcome {
    Running,
    Good,
    Bad,
    Skip,
    Cancelled,
    TimedOut,
    Failed,
    StateChanged,
}
impl BisectTestOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Good => "Good",
            Self::Bad => "Bad",
            Self::Skip => "Skip",
            Self::Cancelled => "Cancelled",
            Self::TimedOut => "Timed out",
            Self::Failed => "Command failed",
            Self::StateChanged => "Checkout changed",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Good => "good",
            Self::Bad => "bad",
            Self::Skip => "skip",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timeout",
            Self::Failed => "failed",
            Self::StateChanged => "changed",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        [
            Self::Running,
            Self::Good,
            Self::Bad,
            Self::Skip,
            Self::Cancelled,
            Self::TimedOut,
            Self::Failed,
            Self::StateChanged,
        ]
        .into_iter()
        .find(|v| v.key() == s)
    }
    fn action(self) -> Option<BisectAction> {
        match self {
            Self::Good => Some(BisectAction::Good),
            Self::Bad => Some(BisectAction::Bad),
            Self::Skip => Some(BisectAction::Skip),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BisectTestResult {
    pub commit: ObjectId,
    pub outcome: BisectTestOutcome,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub recorded: bool,
    /// Bounded local display only. Never included in diagnostic records.
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Default)]
pub struct BisectAutoProgress {
    pub running: Option<ObjectId>,
    pub results: Vec<BisectTestResult>,
}
#[derive(Clone, Debug)]
pub struct BisectAutoPreview {
    step: BisectPreview,
    command: BisectTestCommand,
    program_digest: String,
    history: Vec<u8>,
}
impl BisectAutoPreview {
    pub fn step(&self) -> &BisectPreview {
        &self.step
    }
    pub fn command(&self) -> &BisectTestCommand {
        &self.command
    }
    /// SHA-256 of the reviewed executable; rechecked before every candidate.
    pub fn program_digest(&self) -> &str {
        &self.program_digest
    }
    pub fn worktree(&self) -> &Path {
        self.step
            .worktree
            .as_deref()
            .expect("worktree required at review")
    }
}
#[derive(Debug)]
#[must_use]
pub struct BisectAutoReport {
    pub result: Result<()>,
    pub tests: Vec<BisectTestResult>,
    pub status: Result<Status>,
    pub session: Result<Option<BisectSession>>,
    pub state: Result<OperationState>,
}

fn program_digest(program: &Path, root: &Path, c: &CancellationToken) -> Result<String> {
    let m = fs::symlink_metadata(program)
        .map_err(|e| Error::io("inspect bisect test executable", e))?;
    if !m.is_file() || crate::local_files::redirected(&m) {
        return Err(Error::InvalidInput(
            "Bisect test executable must be a regular local file.".into(),
        ));
    }
    let full =
        fs::canonicalize(program).map_err(|e| Error::io("resolve bisect test executable", e))?;
    let root = fs::canonicalize(root).map_err(|e| Error::io("resolve bisect working folder", e))?;
    if full.starts_with(&root) {
        return Err(Error::InvalidInput("Choose a test executable outside this working folder so candidate checkouts cannot replace the program. Script arguments may refer to the checked-out revision.".into()));
    }
    if m.len() > 128 * 1024 * 1024 || !crate::valid_custom_program_path(&full) {
        return Err(Error::InvalidInput("Choose a regular local test executable no larger than 128 MiB.".into()));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = c; // Only the SHA-256 check below can be cancelled.
        return Err(Error::InvalidInput("Automatic bisect executable verification currently requires Windows or Linux.".into()));
    }
    #[cfg(any(windows, target_os = "linux"))]
    crate::osv_file_digest(&full, 128 * 1024 * 1024, c).map_err(|error| match error {
        Error::Io { source, .. } => Error::io("verify bisect test executable", source),
        Error::InvalidInput(_) => Error::InvalidInput("Could not verify the bisect test executable with SHA-256. Check the selected file (maximum 128 MiB), then review again.".into()),
        other => other,
    })
}
fn history_path(repo: &Repository, saved: &Saved) -> Result<PathBuf> {
    let id = saved
        .recovery
        .strip_prefix("refs/rocket/bisect/")
        .filter(|s| {
            !s.is_empty() && s.len() < 100 && s.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        })
        .ok_or(Error::Bisect(BisectFailure::JournalChanged))?;
    Ok(repo
        .info()
        .git_dir
        .join(format!("rocket-bisect-tests-{id}")))
}
fn history_bytes(path: &Path) -> Result<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(_) => crate::local_files::read_config(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::io("read bisect test results", e)),
    }
}
fn decode(bytes: &[u8]) -> Result<Vec<BisectTestResult>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let invalid = || {
        Error::InvalidInput("Saved bisect test results changed or are incomplete. Keep the file for inspection; manual recovery remains available.".into())
    };
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    if !text.ends_with('\n') {
        return Err(invalid());
    }
    let mut lines = text.lines();
    if lines.next() != Some("ROCKET-BISECT-TESTS-1") {
        return Err(invalid());
    }
    let mut out = Vec::new();
    for line in lines {
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() != 5 || out.len() >= MAX_TESTS {
            return Err(invalid());
        }
        out.push(BisectTestResult {
            commit: parse::oid(fields[0].as_bytes(), "bisect test commit")?,
            outcome: BisectTestOutcome::parse(fields[1]).ok_or_else(invalid)?,
            exit_code: if fields[2] == "-" {
                None
            } else {
                Some(fields[2].parse().map_err(|_| invalid())?)
            },
            elapsed_ms: fields[3].parse().map_err(|_| invalid())?,
            recorded: match fields[4] {
                "0" => false,
                "1" => true,
                _ => return Err(invalid()),
            },
            stdout: Vec::new(),
            stderr: Vec::new(),
            truncated: false,
        });
    }
    Ok(out)
}
fn persist(path: &Path, expected: &mut Vec<u8>, tests: &[BisectTestResult]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(Error::Bisect(BisectFailure::JournalChanged))?;
    let _pin = crate::local_files::pin_directories(parent, parent)?;
    let lock = path.with_extension("lock");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| Error::io("lock bisect test results", e))?;
    let result = (|| {
        if history_bytes(path)? != *expected {
            return fail(BisectFailure::JournalChanged);
        }
        let mut text = String::from("ROCKET-BISECT-TESTS-1\n");
        use std::fmt::Write as _;
        for t in tests {
            writeln!(
                text,
                "{} {} {} {} {}",
                t.commit,
                t.outcome.key(),
                t.exit_code.map_or("-".into(), |n| n.to_string()),
                t.elapsed_ms,
                u8::from(t.recorded)
            )
            .unwrap();
        }
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| Error::io("save bisect test results", e))?;
        drop(file);
        crate::local_files::move_path(&lock, path, !expected.is_empty())?;
        *expected = text.into_bytes();
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&lock);
    }
    result
}
fn publish(
    progress: Option<&Arc<Mutex<BisectAutoProgress>>>,
    tests: &[BisectTestResult],
    running: Option<ObjectId>,
) {
    if let Some(p) = progress {
        let mut p = p.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        p.running = running;
        p.results = tests.to_vec();
    }
}

impl Repository {
    pub fn bisect_test_history(&self, c: &CancellationToken) -> Result<Vec<BisectTestResult>> {
        let Some(session) = self.bisect_session(c)? else {
            return Ok(Vec::new());
        };
        let saved = Saved {
            original: session.original.clone(),
            good: session.good.clone(),
            bad: session.bad.clone(),
            recovery: session.recovery_reference.clone(),
        };
        let mut tests = decode(&history_bytes(&history_path(self, &saved)?)?)?;
        // A decision can complete before interruption prevents updating the result
        // file. Reconcile with Git rather than repeating that completed decision.
        for t in &mut tests {
            if let Some(action) = t.outcome.action() {
                t.recorded = session
                    .decisions
                    .iter()
                    .any(|d| d.commit == t.commit && d.action == action);
            }
        }
        Ok(tests)
    }
    pub fn preview_bisect_auto(
        &self,
        good: &str,
        bad: &str,
        command: &BisectTestCommand,
        c: &CancellationToken,
    ) -> Result<BisectAutoPreview> {
        command.validate()?;
        let active = self.bisect_session(c)?.is_some();
        let step = self.preview_bisect(
            if active {
                BisectAction::Good
            } else {
                BisectAction::Start
            },
            good,
            bad,
            c,
        )?;
        let program_digest = program_digest(
            &command.program,
            step.worktree.as_deref().ok_or(Error::WorktreeRequired)?,
            c,
        )?;
        let history = history_bytes(&history_path(self, &step.saved)?)?;
        let tests = decode(&history)?;
        if tests.len() >= MAX_TESTS {
            return Err(Error::InvalidInput(
                "This bisect reached its 256-test limit. Restore, then choose a smaller range."
                    .into(),
            ));
        }
        c.check()?;
        Ok(BisectAutoPreview {
            step,
            command: command.clone(),
            program_digest,
            history,
        })
    }
    pub fn run_bisect_auto(
        &self,
        p: &BisectAutoPreview,
        storage: &Path,
        progress: Option<&Arc<Mutex<BisectAutoProgress>>>,
        c: &CancellationToken,
    ) -> Result<BisectAutoReport> {
        self.require_worktree(c)?;
        let _permit = self.acquire_write(c)?;
        let mut tests = decode(&p.history)?;
        let mut history = p.history.clone();
        let result=self.runner.step("bisect-auto","run_reviewed_search",||{
            if self.info().git_dir!=p.step.git_dir||self.info().common_dir!=p.step.common_dir||self.info().worktree!=p.step.worktree
                ||self.bisect_context(true,c)?!=p.step.context{return fail(BisectFailure::StalePreview);}
            let log_path=history_path(self,&p.step.saved)?;
            if history_bytes(&log_path)?!=history{return fail(BisectFailure::JournalChanged);}
            if program_digest(&p.command.program,p.worktree(),c)?!=p.program_digest{return Err(Error::InvalidInput("The test executable changed after review. Review the command again.".into()));}
            if p.step.action==BisectAction::Start {self.apply_bisect_locked(&p.step,c)?.result?;}
            loop {
                c.check()?;
                let session=self.bisect_session(c)?.ok_or(Error::Bisect(BisectFailure::NotReady))?;
                if session.recovery_reference!=p.step.saved.recovery{return fail(BisectFailure::ForeignSession);}
                if matches!(session.phase,BisectPhase::Found|BisectPhase::Inconclusive){return Ok(());}
                if !session.can_mark(){return fail(BisectFailure::NotReady);}
                if tests.len()>=MAX_TESTS{return Err(Error::InvalidInput("Stopped after 256 tests. Restore and choose a smaller range; no unfinished test was classified.".into()));}
                let before=self.preview_bisect(BisectAction::Good,"","",c)?;
                self.bisect_original(&before.saved,c)?;
                if program_digest(&p.command.program,p.worktree(),c)?!=p.program_digest{return Err(Error::InvalidInput("Test executable changed between candidates. Review again before running it.".into()));}
                let commit=head_id(&before.context.head)?.clone();
                tests.push(BisectTestResult{commit:commit.clone(),outcome:BisectTestOutcome::Running,exit_code:None,elapsed_ms:0,recorded:false,stdout:Vec::new(),stderr:Vec::new(),truncated:false});
                persist(&log_path,&mut history,&tests)?;
                publish(progress,&tests,Some(commit));
                let started=Instant::now();
                let args:Vec<OsString>=p.command.arguments.iter().map(OsString::from).collect();
                let command=self.run_command_locked(&p.command.program,&args,storage,Some(Duration::from_secs(p.command.timeout_seconds)),c);
                let test=tests.last_mut().unwrap();test.elapsed_ms=started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let exit=match command {Ok(r)=>{
                    test.truncated=r.truncated||r.stdout.len()>OUTPUT_BYTES||r.stderr.len()>OUTPUT_BYTES;
                    test.stdout=r.stdout.into_iter().take(OUTPUT_BYTES).collect();test.stderr=r.stderr.into_iter().take(OUTPUT_BYTES).collect();r.exit
                },Err(e)=>Err(e)};
                let classification=match exit {
                    Ok(Some(code))=>{test.exit_code=Some(code);match code{0=>Ok(BisectTestOutcome::Good),1..=124=>Ok(BisectTestOutcome::Bad),125=>Ok(BisectTestOutcome::Skip),_=>Err(Error::InvalidInput(format!("Test exited with {code}; stopped without classifying this revision. Check the test command, then review again."))) }},
                    Ok(None)=>Err(Error::InvalidInput("Test ended without an exit code; no decision was recorded.".into())),
                    Err(e)=>Err(e),
                };
                let decision=(||{
                    let outcome=classification?;c.check()?;
                    self.bisect_original(&before.saved,c)?;
                    if self.bisect_context(true,c)?!=before.context{return fail(BisectFailure::StalePreview);}
                    Ok(outcome)
                })();
                test.outcome=match &decision {Ok(o)=>*o,Err(Error::Cancelled)=>BisectTestOutcome::Cancelled,Err(Error::TimedOut)=>BisectTestOutcome::TimedOut,Err(Error::Bisect(_))=>BisectTestOutcome::StateChanged,Err(_)=>BisectTestOutcome::Failed};
                persist(&log_path,&mut history,&tests)?;publish(progress,&tests,None);
                if let Some(log) = &self.runner.log { log.bisect_test(tests.last().unwrap(), tests.len()); }
                let outcome=decision?;
                let mut next=before;next.action=outcome.action().expect("classified exit");
                let applied=self.apply_bisect_locked(&next,c)?;
                let recorded=applied.session.as_ref().ok().and_then(|s|s.as_ref()).is_some_and(|s|s.decisions.iter().any(|d|d.commit==tests.last().unwrap().commit&&Some(d.action)==outcome.action()));
                tests.last_mut().unwrap().recorded=recorded;
                persist(&log_path,&mut history,&tests)?;publish(progress,&tests,None);
                applied.result?;
            }
        });
        publish(progress, &tests, None);
        let fresh = CancellationToken::new();
        Ok(BisectAutoReport {
            result,
            tests,
            status: self.status(&fresh),
            session: self.bisect_session(&fresh),
            state: self.operation_state(&fresh),
        })
    }
}
