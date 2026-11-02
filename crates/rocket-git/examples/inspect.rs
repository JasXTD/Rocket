use rocket_git::{
    BranchWriteReport, CancellationToken, DiagnosticLog, DiffOptions, DiffTarget, Error,
    FetchProgress, FetchReport, GitClient, HistoryStart, MergeMode, MergeWriteReport, ObjectId,
    PullReport, PushOptions, PushRemoteOutcome, PushReport, RepoPath, Result, StashWriteReport,
    WriteReport, WriteTask,
};
use std::{ffi::OsString, io, path::PathBuf};

#[macro_use]
#[path = "support/terminal.rs"]
mod terminal;

#[path = "support/clone.rs"]
mod clone;

const USAGE: &str = "usage: inspect <existing-repository> [info|status|branches|history|diff|staged]\n\
    inspect <existing-repository> stage <literal-path>...\n\
    inspect <existing-repository> unstage <literal-path>...\n\
    inspect <existing-repository> stage-all|unstage-all\n\
    inspect <existing-repository> commit -m \"message\"\n\
    inspect <existing-repository> branch-create <name> [start-commit-or-ref]\n\
    inspect <existing-repository> branch-switch <name>\n\
    inspect <existing-repository> branch-rename <old-name> <new-name>\n\
    inspect <existing-repository> branch-delete <name>\n\
    inspect <existing-repository> stashes|operation\n\
    inspect <existing-repository> stash-push [-u] [-m \"message\"]\n\
    inspect <existing-repository> stash-apply <full-stash-id> [--index]\n\
    inspect <existing-repository> stash-drop <index> <expected-full-stash-id>\n\
    inspect <existing-repository> merge <target> [--ff-only|--no-ff]\n\
    inspect <existing-repository> merge-continue -m \"message\"\n\
    inspect <existing-repository> merge-abort\n\
    inspect <existing-repository> remotes\n\
    inspect <existing-repository> fetch <configured-remote-name> [--raw-error]\n\
    inspect <existing-repository> pull [--raw-error]\n\
    inspect <existing-repository> push <remote-name> <local-branch> <remote-branch> [--set-upstream] [--raw-error]\n\
    inspect --log-path\n\
    inspect clone --local <source-directory> <new-destination> [--raw-error] [--cancel-after-ms N]\n\
    inspect clone --url <https://... or ssh://...> <new-destination> [--raw-error] [--cancel-after-ms N]\n\
Diagnostics default to the user log directory. ROCKET_LOG_DIR overrides it; ROCKET_LOG=off disables logs.\n\
Repository-operation paths are relative to the repository root; clone paths use the current directory.\n\
Write commands modify the chosen repository or create a new clone destination.";

fn no_arguments(arguments: &[OsString]) -> Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "unexpected arguments\n{USAGE}"
        )))
    }
}

fn argument_bytes(value: &OsString) -> Result<Vec<u8>> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = value
        .to_str()
        .ok_or_else(|| Error::InvalidInput("argument is not UTF-8".into()))?
        .as_bytes()
        .to_vec();
    Ok(bytes)
}

fn repo_path(path: &OsString) -> Result<RepoPath> {
    let bytes = argument_bytes(path)?;
    #[cfg(not(unix))]
    let bytes = bytes
        .into_iter()
        .map(|byte| if byte == b'\\' { b'/' } else { byte })
        .collect::<Vec<_>>();
    RepoPath::new(bytes)
}

fn show_write(report: WriteReport) -> Result<()> {
    match &report.result {
        Ok(output) => {
            print!("{}", String::from_utf8_lossy(&output.stdout));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            println!("{:?}: Git command succeeded.", report.operation);
        }
        Err(_) => eprintln!(
            "{:?}: command failed or was interrupted. Inspect the refreshed status before retrying.",
            report.operation
        ),
    }
    match report.status {
        Ok(status) => println!("Status after the command:\n{status:#?}"),
        Err(error) => {
            eprintln!("Status refresh failed: {error}. The command result above still applies.")
        }
    }
    // Refresh failure must not turn a confirmed successful commit into a failed command.
    report.result.map(|_| ())
}

fn show_branch_write(report: BranchWriteReport) -> Result<()> {
    // Report every observation even when one read or the command itself failed.
    let result = show_write(report.write);
    match report.branches {
        Ok(branches) => println!("Branches after the command:\n{branches:#?}"),
        Err(error) => {
            eprintln!("Branch refresh failed: {error}. The command result above still applies.")
        }
    }
    result
}

fn show_stashes(stashes: Vec<rocket_git::StashEntry>) {
    for stash in stashes {
        println!(
            "{} {} {}",
            stash.index,
            stash.id,
            String::from_utf8_lossy(&stash.subject)
        );
    }
}

fn show_stash_write(report: StashWriteReport) -> Result<()> {
    let result = show_write(report.write);
    match report.stashes {
        Ok(stashes) => {
            println!("Stashes after the command:");
            show_stashes(stashes);
        }
        Err(error) => {
            eprintln!("Stash refresh failed: {error}. The command result above still applies.")
        }
    }
    result
}

fn show_merge_write(report: MergeWriteReport) -> Result<()> {
    let result = show_branch_write(BranchWriteReport {
        write: report.write,
        branches: report.branches,
    });
    match report.state {
        Ok(state) => println!("Operation state after the command:\n{state:#?}"),
        Err(error) => {
            eprintln!("Operation refresh failed: {error}. The command result above still applies.")
        }
    }
    result
}

fn show_fetch(report: FetchReport, raw_error: bool) -> Result<()> {
    match &report.result {
        Ok(output) => {
            eprint!("{}", String::from_utf8_lossy(&output.output.stderr));
            println!("Fetch: Git command succeeded.");
            match &output.updates {
                Ok(updates) => println!("Ref updates:\n{updates:#?}"),
                Err(error) => {
                    eprintln!("Fetch succeeded, but parsing its ref updates failed: {error}")
                }
            }
        }
        Err(error) => {
            eprintln!(
                "Fetch failed or was interrupted. Inspect the refreshed state before retrying."
            );
            if raw_error && let Error::Git { stdout, stderr, .. } = error {
                // Explicit local inspection only. Escaping prevents terminal
                // controls but does not redact secrets; never send this to logs.
                eprintln!("Raw Git output (may contain credentials; escaped for display):");
                eprintln!("stdout: {:?}", String::from_utf8_lossy(stdout));
                eprintln!("stderr: {:?}", String::from_utf8_lossy(stderr));
            }
            match error {
                Error::TimedOut => eprintln!(
                    "The command exceeded its time limit. Check connectivity and complete any credential-helper sign-in or key unlock separately before retrying."
                ),
                Error::Cancelled => eprintln!(
                    "Fetch was cancelled. Downloaded objects or ref changes may already exist; cancellation is not rollback."
                ),
                _ => (),
            }
        }
    }
    match report.branches {
        Ok(branches) => println!("Branches after fetch:\n{branches:#?}"),
        Err(error) => eprintln!("Branch refresh failed: {error}. The fetch result still applies."),
    }
    match report.status {
        Some(Ok(status)) => println!("Status after fetch:\n{status:#?}"),
        Some(Err(error)) => {
            eprintln!("Status refresh failed: {error}. The fetch result still applies.")
        }
        None => println!("Bare repository: no worktree status."),
    }
    report.result.map(|_| ())
}

fn show_pull(report: PullReport, raw_error: bool) -> Result<()> {
    match &report.fetch {
        Ok(output) => {
            println!("Pull fetch: Git command succeeded.");
            match &output.updates {
                Ok(updates) => println!("Fetched ref updates:\n{updates:#?}"),
                Err(error) => eprintln!("Fetch succeeded, but parsing its updates failed: {error}"),
            }
        }
        Err(_) => eprintln!("Pull fetch: failed or interrupted; branch update was not attempted."),
    }
    match &report.update {
        Some(Ok(update)) => {
            println!(
                "Pull branch update: {:?} (Git command succeeded).",
                update.kind
            );
            if let Err(error) = &update.verification {
                eprintln!(
                    "Branch verification failed: {error}. The successful command still applies."
                );
            }
            if update.error_diagnostics {
                eprintln!(
                    "Git emitted error diagnostics despite a successful branch update. Inspect retained output and refreshed state before retrying."
                );
            }
        }
        Some(Err(_)) => eprintln!(
            "Pull branch update: refused, failed, or interrupted. Fetch already succeeded; inspect refreshed state before retrying."
        ),
        None => println!("Pull branch update: not attempted."),
    }
    if raw_error {
        let error = report
            .fetch
            .as_ref()
            .err()
            .or_else(|| report.update.as_ref().and_then(|r| r.as_ref().err()));
        if let Some(Error::Git { stdout, stderr, .. }) = error {
            eprintln!("Raw Git output (may contain credentials; escaped for display):");
            eprintln!("stdout: {:?}", String::from_utf8_lossy(stdout));
            eprintln!("stderr: {:?}", String::from_utf8_lossy(stderr));
        }
        if let Some(Ok(update)) = &report.update {
            eprintln!("Raw branch-update output (may contain private data; escaped for display):");
            eprintln!(
                "stdout: {:?}",
                String::from_utf8_lossy(&update.output.stdout)
            );
            eprintln!(
                "stderr: {:?}",
                String::from_utf8_lossy(&update.output.stderr)
            );
        }
    }
    match report.status {
        Ok(status) => println!("Status after pull:\n{status:#?}"),
        Err(error) => eprintln!("Status refresh failed: {error}. Pull phase results still apply."),
    }
    match report.branches {
        Ok(branches) => println!("Branches after pull:\n{branches:#?}"),
        Err(error) => eprintln!("Branch refresh failed: {error}. Pull phase results still apply."),
    }
    match report.state {
        Ok(state) => println!("Operation state after pull:\n{state:#?}"),
        Err(error) => {
            eprintln!("Operation refresh failed: {error}. Pull phase results still apply.")
        }
    }
    let fetched = report.fetch?;
    fetched.updates?;
    report
        .update
        .ok_or(Error::Pull(rocket_git::PullFailure::UpstreamNotFetched))?
        .map(|_| ())
}

fn wait_for_fetch_progress<T: Send + 'static>(
    mut task: WriteTask<T>,
    receiver: std::sync::mpsc::Receiver<FetchProgress>,
) -> Result<T> {
    loop {
        for update in receiver.try_iter() {
            match update.total {
                Some(total) => eprintln!("Fetch {:?}: {}/{total}", update.phase, update.current),
                None => eprintln!("Fetch {:?}: {}", update.phase, update.current),
            }
        }
        if let Some(result) = task.try_finish() {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn text_argument(value: &OsString) -> Result<&str> {
    value
        .to_str()
        .ok_or_else(|| Error::InvalidInput("argument must be UTF-8".into()))
}

fn show_push(report: PushReport, raw_error: bool) -> Result<()> {
    if report.result.is_ok() {
        println!("Push: Git command succeeded.");
    } else {
        eprintln!("Push: command failed or was interrupted.");
    }
    println!("Remote outcome: {}", report.remote_outcome.code());
    if report.error_diagnostics && report.remote_outcome == PushRemoteOutcome::Confirmed {
        eprintln!(
            "Git emitted error diagnostics despite remote success. Inspect local tracking/upstream state and retained Git output before retrying."
        );
    }
    match report.remote_outcome {
        PushRemoteOutcome::NotAttempted => eprintln!(
            "Push did not start; no remote update was sent. Resolve the local refusal before retrying."
        ),
        PushRemoteOutcome::Unknown => eprintln!(
            "Remote update is unconfirmed. Local refreshes cannot tell whether the server changed. Check the remote branch before retrying; interruption is not rollback."
        ),
        PushRemoteOutcome::Rejected => eprintln!(
            "Git reported a rejection for the selected branch. Inspect the reason below before retrying."
        ),
        PushRemoteOutcome::Confirmed if report.result.is_err() => eprintln!(
            "Git reported remote success despite a command error. Inspect local tracking/upstream state; do not repeat the write automatically."
        ),
        _ => (),
    }
    match report.update {
        Some(Ok(update)) => println!("Push update:\n{update:#?}"),
        Some(Err(error)) => eprintln!(
            "Push status parsing failed: {error}. The command and remote outcome above still apply."
        ),
        None => (),
    }
    if raw_error {
        let raw = match &report.result {
            Ok(output) => Some((&output.stdout, &output.stderr)),
            Err(Error::Git { stdout, stderr, .. }) => Some((stdout, stderr)),
            _ => None,
        };
        if let Some((stdout, stderr)) = raw {
            eprintln!("Raw Git output (may contain credentials; escaped for display):");
            eprintln!("stdout: {:?}", String::from_utf8_lossy(stdout));
            eprintln!("stderr: {:?}", String::from_utf8_lossy(stderr));
        }
    }
    if let Some(upstream) = report.upstream {
        match upstream {
            Ok(()) => println!("Upstream: configured and verified."),
            Err(error) => {
                eprintln!("Upstream verification failed: {error}. Remote success is unchanged.")
            }
        }
    }
    match report.branches {
        Ok(branches) => println!("Branches after push:\n{branches:#?}"),
        Err(error) => eprintln!("Branch refresh failed: {error}. The push outcome still applies."),
    }
    match report.status {
        Some(Ok(status)) => println!("Status after push:\n{status:#?}"),
        Some(Err(error)) => {
            eprintln!("Status refresh failed: {error}. The push outcome still applies.")
        }
        None => println!("Bare repository: no worktree status."),
    }
    report.result.map(|_| ())
}

fn run(mut arguments: impl Iterator<Item = OsString>, log: Option<&DiagnosticLog>) -> Result<()> {
    let Some(directory) = arguments.next() else {
        return Err(Error::InvalidInput(USAGE.into()));
    };
    if directory == "clone" {
        return clone::run(arguments, log);
    }
    let operation = arguments.next().unwrap_or_else(|| "status".into());
    let remaining: Vec<_> = arguments.collect();
    let cancel = CancellationToken::new();
    let git = match log {
        Some(log) => GitClient::discover_with_log(log.clone())?,
        None => GitClient::discover()?,
    };
    let repo = git.open(directory, &cancel)?;
    match operation.to_str() {
        Some("info") => {
            no_arguments(&remaining)?;
            println!("{:#?}", repo.info());
        }
        Some("status") => {
            no_arguments(&remaining)?;
            println!("{:#?}", repo.status(&cancel)?);
        }
        Some("branches") => {
            no_arguments(&remaining)?;
            println!("{:#?}", repo.branches(&cancel)?);
        }
        Some("remotes") => {
            no_arguments(&remaining)?;
            for remote in repo.remotes(&cancel)? {
                // Debug escaping keeps control characters from being interpreted
                // by the terminal. Full configured URLs may include credentials.
                println!("Remote {:?}", String::from_utf8_lossy(&remote.name));
                for url in remote.urls {
                    println!("  fetch URL: {:?}", String::from_utf8_lossy(&url));
                }
                for url in remote.push_urls {
                    println!("  push URL: {:?}", String::from_utf8_lossy(&url));
                }
                for spec in remote.fetch_refspecs {
                    println!("  fetch mapping: {:?}", String::from_utf8_lossy(&spec));
                }
            }
        }
        Some("fetch") => {
            let (name, raw_error) = match remaining.as_slice() {
                [name] => (name, false),
                [name, flag] if flag == "--raw-error" => (name, true),
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            let name = argument_bytes(name)?;
            let (sender, receiver) = std::sync::mpsc::sync_channel(32);
            let task = WriteTask::spawn(move |cancel| repo.fetch(name, Some(&sender), &cancel))?;
            show_fetch(wait_for_fetch_progress(task, receiver)?, raw_error)?;
        }
        Some("pull") => {
            let raw_error = match remaining.as_slice() {
                [] => false,
                [flag] if flag == "--raw-error" => true,
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            let (sender, receiver) = std::sync::mpsc::sync_channel(32);
            let task = WriteTask::spawn(move |cancel| repo.pull_ff_only(Some(&sender), &cancel))?;
            show_pull(wait_for_fetch_progress(task, receiver)?, raw_error)?;
        }
        Some("push") => {
            let [remote, local, destination, flags @ ..] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            let mut options = PushOptions::default();
            let mut raw_error = false;
            for flag in flags {
                if flag == "--set-upstream" && !options.set_upstream {
                    options.set_upstream = true;
                } else if flag == "--raw-error" && !raw_error {
                    raw_error = true;
                } else {
                    return Err(Error::InvalidInput(USAGE.into()));
                }
            }
            let (remote, local, destination) = (
                argument_bytes(remote)?,
                argument_bytes(local)?,
                argument_bytes(destination)?,
            );
            let task = WriteTask::spawn(move |cancel| {
                repo.push(remote, local, destination, options, &cancel)
            })?;
            show_push(task.wait()?, raw_error)?;
        }
        Some("stashes") => {
            no_arguments(&remaining)?;
            show_stashes(repo.stashes(&cancel)?);
        }
        Some("operation") => {
            no_arguments(&remaining)?;
            println!("{:#?}", repo.operation_state(&cancel)?);
        }
        Some("stash-push") => {
            let (include_untracked, message) = match remaining.as_slice() {
                [] => (false, None),
                [flag] if flag == "-u" => (true, None),
                [flag, message] if flag == "-m" => (false, Some(text_argument(message)?)),
                [untracked, flag, message] if untracked == "-u" && flag == "-m" => {
                    (true, Some(text_argument(message)?))
                }
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            show_stash_write(repo.push_stash(message, include_untracked, &cancel)?)?;
        }
        Some("stash-apply") => {
            let (id, restore_index) = match remaining.as_slice() {
                [id] => (id, false),
                [id, flag] if flag == "--index" => (id, true),
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            show_stash_write(repo.apply_stash(
                &ObjectId::parse(text_argument(id)?)?,
                restore_index,
                &cancel,
            )?)?;
        }
        Some("stash-drop") => {
            let [index, expected] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            let index: usize = text_argument(index)?.parse().map_err(|_| {
                Error::InvalidInput("stash index must be a nonnegative integer".into())
            })?;
            let expected = ObjectId::parse(text_argument(expected)?)?;
            let stashes = repo.stashes(&cancel)?;
            let entry = stashes
                .get(index)
                .filter(|entry| entry.id == expected)
                .ok_or_else(|| {
                    Error::InvalidInput("stash selection changed; list stashes again".into())
                })?;
            show_stash_write(repo.drop_stash(entry, &cancel)?)?;
        }
        Some("merge") => {
            let (target, mode) = match remaining.as_slice() {
                [target] => (target, MergeMode::AllowFastForward),
                [target, flag] if flag == "--ff-only" => (target, MergeMode::FastForwardOnly),
                [target, flag] if flag == "--no-ff" => (target, MergeMode::NoFastForward),
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            show_merge_write(repo.merge(text_argument(target)?, mode, &cancel)?)?;
        }
        Some("merge-continue") => {
            let [flag, message] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            if flag != "-m" {
                return Err(Error::InvalidInput(USAGE.into()));
            }
            show_merge_write(repo.continue_merge(text_argument(message)?, &cancel)?)?;
        }
        Some("merge-abort") => {
            no_arguments(&remaining)?;
            show_merge_write(repo.abort_merge(&cancel)?)?;
        }
        Some("history") => {
            no_arguments(&remaining)?;
            let page = repo.history(HistoryStart::Head, 25, &cancel)?;
            for commit in page.commits {
                println!("{} {}", commit.id, commit.subject_lossy());
            }
            if page.next.is_some() {
                println!("(more history available through the library cursor)");
            }
        }
        Some("diff" | "staged") => {
            if remaining.len() > 1 {
                return Err(Error::InvalidInput(
                    "diff takes at most one literal path".into(),
                ));
            }
            let mut options = DiffOptions {
                target: if operation == "staged" {
                    DiffTarget::Staged
                } else {
                    DiffTarget::Unstaged
                },
                ..Default::default()
            };
            if let Some(path) = remaining.first() {
                options.paths.push(repo_path(path)?);
            }
            use std::io::Write;
            std::io::stdout()
                .write_all(terminal::safe(&String::from_utf8_lossy(&repo.diff(&options, &cancel)?.patch)).as_bytes())
                .map_err(|source| rocket_git::Error::Io {
                    action: "write patch",
                    source,
                })?;
        }
        Some("stage" | "unstage") => {
            let paths = remaining
                .iter()
                .map(repo_path)
                .collect::<Result<Vec<_>>>()?;
            let report = if operation == "stage" {
                repo.stage(&paths, &cancel)?
            } else {
                repo.unstage(&paths, &cancel)?
            };
            show_write(report)?;
        }
        Some("stage-all") => {
            no_arguments(&remaining)?;
            show_write(repo.stage_all(&cancel)?)?;
        }
        Some("unstage-all") => {
            no_arguments(&remaining)?;
            show_write(repo.unstage_all(&cancel)?)?;
        }
        Some("commit") => {
            let [flag, message] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            if flag != "-m" {
                return Err(Error::InvalidInput(USAGE.into()));
            }
            let message = message
                .to_str()
                .ok_or_else(|| Error::InvalidInput("commit message must be UTF-8".into()))?;
            show_write(repo.commit(message, &cancel)?)?;
        }
        Some("branch-create") => {
            let (name, start) = match remaining.as_slice() {
                [name] => (name, None),
                [name, start] => (
                    name,
                    Some(start.to_str().ok_or_else(|| {
                        Error::InvalidInput("start revision must be UTF-8".into())
                    })?),
                ),
                _ => return Err(Error::InvalidInput(USAGE.into())),
            };
            show_branch_write(repo.create_branch(argument_bytes(name)?, start, &cancel)?)?;
        }
        Some("branch-switch" | "branch-delete") => {
            let [name] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            let name = argument_bytes(name)?;
            let report = if operation == "branch-switch" {
                repo.switch_branch(name, &cancel)?
            } else {
                repo.delete_branch(name, &cancel)?
            };
            show_branch_write(report)?;
        }
        Some("branch-rename") => {
            let [old, new] = remaining.as_slice() else {
                return Err(Error::InvalidInput(USAGE.into()));
            };
            show_branch_write(repo.rename_branch(
                argument_bytes(old)?,
                argument_bytes(new)?,
                &cancel,
            )?)?;
        }
        _ => {
            return Err(rocket_git::Error::InvalidInput(format!(
                "unknown operation\n{USAGE}"
            )));
        }
    }
    Ok(())
}

fn log_directory() -> io::Result<PathBuf> {
    let directory = if let Some(path) = std::env::var_os("ROCKET_LOG_DIR") {
        PathBuf::from(path)
    } else {
        #[cfg(windows)]
        let base = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Rocket/logs"));
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Logs/Rocket"));
        #[cfg(all(not(windows), not(target_os = "macos")))]
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(|p| PathBuf::from(p).join("rocket/logs"))
            .or_else(|| {
                std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state/rocket/logs"))
            });
        base.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "user log directory unavailable; set ROCKET_LOG_DIR",
            )
        })?
    };
    if !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ROCKET_LOG_DIR and user directory must be absolute",
        ));
    }
    Ok(directory)
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.as_slice() == [OsString::from("--log-path")] {
        match log_directory() {
            Ok(directory) => println!("{}", directory.join("rocket.jsonl").display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let log = if std::env::var_os("ROCKET_LOG").is_some_and(|value| value == "off") {
        None
    } else {
        match log_directory().and_then(DiagnosticLog::open) {
            Ok(log) => {
                eprintln!(
                    "Diagnostics: {} (session {})",
                    log.directory().join("rocket.jsonl").display(),
                    log.session_id()
                );
                Some(log)
            }
            Err(error) => {
                eprintln!(
                    "Warning: diagnostic logging unavailable: {error}. Git operations can still run."
                );
                None
            }
        }
    };
    let result = match &log {
        Some(log) => {
            let operation = if arguments.first().is_some_and(|a| a == "clone") {
                "clone"
            } else {
                arguments
                    .get(1)
                    .map_or("status", |value| value.to_str().unwrap_or("other"))
            };
            log.request(operation, |context| {
                run(arguments.iter().cloned(), Some(context))
            })
        }
        None => run(arguments.into_iter(), None),
    };
    if let Some(log) = &log {
        let health = log.health();
        if health.dropped_events > 0 {
            eprintln!(
                "Warning: {} diagnostic events were not saved ({:?}, OS error {:?}). Git result is unchanged.",
                health.dropped_events, health.last_error_kind, health.last_os_error
            );
        }
    }
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
