use rocket_git::{
    CloneProgress, CloneReport, CloneSource, CommandLimits, DiagnosticLog, Error, GitClient,
    Result, WriteTask,
};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

const USAGE: &str = "usage: inspect clone --local <source-directory> <new-destination> [--raw-error] [--cancel-after-ms N]\n       inspect clone --url <https://... or ssh://...> <new-destination> [--raw-error] [--cancel-after-ms N]";

pub fn run(arguments: impl Iterator<Item = OsString>, log: Option<&DiagnosticLog>) -> Result<()> {
    let mut arguments = arguments;
    let kind = arguments
        .next()
        .ok_or_else(|| Error::InvalidInput(USAGE.into()))?;
    let source = arguments
        .next()
        .ok_or_else(|| Error::InvalidInput(USAGE.into()))?;
    let destination = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| Error::InvalidInput(USAGE.into()))?,
    );
    let source = match kind.to_str() {
        Some("--local") => CloneSource::Local(source.into()),
        Some("--url") => CloneSource::Url(
            source
                .into_string()
                .map_err(|_| Error::InvalidInput("clone URL must be UTF-8".into()))?,
        ),
        _ => return Err(Error::InvalidInput(USAGE.into())),
    };
    let mut raw = false;
    let mut cancel_after = None;
    while let Some(option) = arguments.next() {
        match option.to_str() {
            Some("--raw-error") if !raw => raw = true,
            Some("--cancel-after-ms") if cancel_after.is_none() => {
                cancel_after = Some(Duration::from_millis(
                    arguments
                        .next()
                        .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()))
                        .ok_or_else(|| {
                            Error::InvalidInput(
                                "--cancel-after-ms requires a nonnegative integer".into(),
                            )
                        })?,
                ));
            }
            _ => return Err(Error::InvalidInput(USAGE.into())),
        }
    }
    // Clones can take longer than repository reads. This remains a bounded CLI
    // policy; embedders choose their own limits through GitClient::new.
    let discovered = GitClient::discover()?;
    let limits = CommandLimits {
        timeout: Duration::from_secs(600),
        ..CommandLimits::default()
    };
    let git = match log {
        Some(log) => GitClient::new_with_log(discovered.executable(), limits, log.clone())?,
        None => GitClient::new(discovered.executable(), limits)?,
    };
    let _interrupts = interrupts::Guard::install()?;
    let (sender, receiver) = mpsc::sync_channel(128);
    let mut task = WriteTask::spawn(move |cancel| {
        git.clone_repository(&source, destination, Some(&sender), &cancel)
    })?;
    eprintln!(
        "Clone started. Ctrl+C requests cancellation; wait for the final path/outcome report."
    );
    let started = Instant::now();
    let mut cancellation_sent = false;
    loop {
        if !cancellation_sent
            && (interrupts::requested()
                || cancel_after.is_some_and(|after| started.elapsed() >= after))
        {
            cancellation_sent = true;
            task.cancel();
            eprintln!("Cancellation requested; waiting for Git and its helpers to stop.");
        }
        for progress in receiver.try_iter() {
            show_progress(progress);
        }
        if let Some(result) = task.try_finish() {
            for progress in receiver.try_iter() {
                show_progress(progress);
            }
            return show_report(result?, raw);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn show_progress(progress: CloneProgress) {
    match progress {
        CloneProgress::Phase(phase) => eprintln!("Clone phase: {phase:?}"),
        CloneProgress::Transfer(value) => eprintln!(
            "Clone {:?}: {} / {:?} objects",
            value.phase, value.current, value.total
        ),
        CloneProgress::Checkout { current, total } => {
            eprintln!("Clone checkout: {current} / {total:?} files")
        }
        CloneProgress::Files { done, total } => {
            eprintln!("Clone file download: {done} / {total} files")
        }
    }
}

fn show_report(report: CloneReport, raw: bool) -> Result<()> {
    fn state<T>(value: &Option<Result<T>>) -> &'static str {
        match value {
            None => "skipped",
            Some(Ok(_)) => "succeeded",
            Some(Err(_)) => "failed",
        }
    }
    println!(
        "Clone command: {}",
        if report.command.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
    );
    println!("Clone verification: {}", state(&report.verification));
    println!("Clone publication: {}", state(&report.publication));
    if report.command.is_ok() && report.error_diagnostics {
        eprintln!(
            "Warning: Git emitted error diagnostics despite a successful clone command. Inspect the phase results; raw output is available with --raw-error."
        );
    }
    if matches!(report.publication, Some(Ok(()))) {
        println!("Clone published at {:?}", report.destination);
        if let Some(Err(error)) = &report.repository {
            eprintln!("Warning: reopening the published clone failed: {error}");
            if raw {
                show_raw(error);
            }
        }
        if let Some(Err(error)) = &report.cleanup {
            eprintln!(
                "Warning: {error}. Extra workspace retained at {:?}",
                report.staging_directory
            );
        }
    } else {
        eprintln!(
            "Clone was not published. Workspace retained for inspection: {:?}",
            report.staging_directory
        );
        eprintln!(
            "Git may have removed its incomplete checkout. Rocket does not recursively delete this workspace or retry automatically."
        );
    }
    if raw {
        for error in report
            .command
            .as_ref()
            .err()
            .into_iter()
            .chain(report.verification.as_ref().and_then(|r| r.as_ref().err()))
            .chain(report.publication.as_ref().and_then(|r| r.as_ref().err()))
        {
            show_raw(error);
        }
        if let Ok(output) = &report.command {
            eprintln!(
                "Raw clone stdout (may contain private data): {:?}",
                String::from_utf8_lossy(&output.stdout)
            );
            eprintln!(
                "Raw clone stderr (may contain private data): {:?}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    report.command?;
    if let Some(result) = report.verification {
        result?;
    }
    if let Some(result) = report.publication {
        result?;
    }
    // Successful publication is not undone by a later reopen/empty-cleanup error.
    Ok(())
}

fn show_raw(error: &Error) {
    if let Error::Git { stdout, stderr, .. } = error {
        eprintln!(
            "Raw clone stdout (may contain private data): {:?}",
            String::from_utf8_lossy(stdout)
        );
        eprintln!(
            "Raw clone stderr (may contain private data): {:?}",
            String::from_utf8_lossy(stderr)
        );
    }
}

#[cfg(windows)]
mod interrupts {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    static REQUESTED: AtomicBool = AtomicBool::new(false);
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    unsafe extern "system" fn handler(event: u32) -> i32 {
        if event == 0 || event == 1 {
            REQUESTED.store(true, Ordering::Release);
            1
        } else {
            0
        }
    }
    pub struct Guard;
    impl Guard {
        pub fn install() -> Result<Self> {
            REQUESTED.store(false, Ordering::Release);
            // SAFETY: the function pointer has the documented WINAPI ABI and static
            // lifetime. The callback performs only an atomic store, never I/O or locks.
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                return Err(Error::Io {
                    action: "install clone cancellation handler",
                    source: std::io::Error::last_os_error(),
                });
            }
            Ok(Self)
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: unregister the same live callback after report delivery.
            if unsafe { SetConsoleCtrlHandler(Some(handler), 0) } == 0 {
                eprintln!(
                    "Warning: clone cancellation handler could not be removed: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
    pub fn requested() -> bool {
        REQUESTED.load(Ordering::Acquire)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn console_callback_requests_cancellation_and_ignores_other_events() {
            let _guard = Guard::install().unwrap();
            assert!(!requested());
            // SAFETY: exercise the pure callback with documented console event codes.
            unsafe {
                assert_eq!(handler(2), 0);
                assert_eq!(handler(0), 1);
            }
            assert!(requested());
        }
    }
}

#[cfg(not(windows))]
mod interrupts {
    use super::*;
    pub struct Guard;
    impl Guard {
        pub fn install() -> Result<Self> {
            Err(Error::Unsupported(
                "clone CLI currently requires Windows".into(),
            ))
        }
    }
    pub fn requested() -> bool {
        false
    }
}
