//! Opt-in acceptance probe. Used by work/smoke-public-https-clone.ps1; cargo test
//! does not run main or contact the network. Uses only Rocket and the standard library.
use rocket_git::{
    CloneProgress, CloneSource, CommandLimits, DiagnosticLog, Error, FetchPhase, GitClient, Result,
    WriteTask,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::InvalidInput(message.into()))
    }
}

fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    require(
        arguments.len() == 3,
        "usage: clone_cancel_probe <--local|--url> <source> <new-destination>",
    )?;
    let source = match arguments[0].to_str() {
        Some("--local") => CloneSource::Local(PathBuf::from(&arguments[1])),
        Some("--url") => CloneSource::Url(
            arguments[1]
                .to_str()
                .ok_or_else(|| Error::InvalidInput("URL must be UTF-8".into()))?
                .into(),
        ),
        _ => return Err(Error::InvalidInput("expected --local or --url".into())),
    };
    let destination = PathBuf::from(&arguments[2]);
    let logs = std::env::var_os("ROCKET_LOG_DIR")
        .ok_or_else(|| Error::InvalidInput("set an isolated ROCKET_LOG_DIR".into()))?;
    let log = DiagnosticLog::open(PathBuf::from(logs)).map_err(|source| Error::Io {
        action: "open acceptance diagnostics",
        source,
    })?;
    let mut accepted = false;
    let result: Result<()> = log.request("clone", |context| {
        let discovered = GitClient::discover()?;
        let git = GitClient::new_with_log(
            discovered.executable(),
            CommandLimits {
                timeout: Duration::from_secs(120),
                ..CommandLimits::default()
            },
            context.clone(),
        )?;
        let (sender, receiver) = mpsc::sync_channel(128);
        let target = destination.clone();
        let task = WriteTask::spawn(move |cancel| {
            git.clone_repository(&source, target, Some(&sender), &cancel)
        })?;
        let started = Instant::now();
        let mut trigger = None;
        // Cancel only after real Receiving progress, before its total is reached.
        // A completed/failed/too-fast clone is a failed probe, never a cancel pass.
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(CloneProgress::Transfer(value))
                    if value.phase == FetchPhase::Receiving
                        && value.current > 0
                        && value.total.is_some_and(|total| value.current < total) =>
                {
                    task.cancel();
                    trigger = Some(Instant::now());
                    println!(
                        "CANCEL_REQUEST receiving_current={} receiving_total={} elapsed_ms={}",
                        value.current,
                        value.total.unwrap(),
                        started.elapsed().as_millis()
                    );
                    break;
                }
                Ok(CloneProgress::Phase(phase)) => println!("Clone phase: {phase:?}"),
                Ok(_) | Err(RecvTimeoutError::Timeout) => (),
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if started.elapsed() >= Duration::from_secs(120) {
                task.cancel();
                break;
            }
        }
        // A completion barrier: retain the report and let the worker reap helpers.
        let report = task.wait()?;
        let stop_ms = trigger.map(|time| time.elapsed().as_millis());
        require(trigger.is_some(), "no in-flight Receiving event; cancellation not proven")?;
        require(
            matches!(report.command, Err(Error::Cancelled)),
            "clone command did not report cancellation",
        )?;
        require(
            report.verification.is_none()
                && report.publication.is_none()
                && report.repository.is_none()
                && report.cleanup.is_none(),
            "cancelled clone unexpectedly proceeded past its command",
        )?;
        require(
            std::fs::symlink_metadata(&destination)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "cancelled clone destination exists or cannot be inspected",
        )?;
        require(report.staging_directory.is_dir(), "cancelled workspace was not retained")?;
        require(stop_ms.is_some_and(|ms| ms < 10_000), "cancellation exceeded ten seconds")?;
        println!(
            "CANCEL_REPORT stop_ms={} destination_absent=true workspace_retained=true verification=skipped publication=skipped",
            stop_ms.unwrap()
        );
        // The acceptance process succeeds, while diagnostics correctly describe
        // the operation itself as cancelled rather than a successful clone.
        accepted = true;
        Err(Error::Cancelled)
    });
    require(
        log.health().dropped_events == 0,
        "acceptance diagnostics dropped events",
    )?;
    if accepted && matches!(result, Err(Error::Cancelled)) {
        println!("PASS cancellation after receiving progress");
        Ok(())
    } else {
        result
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Cancellation probe failed: {error}");
        std::process::exit(1);
    }
}
