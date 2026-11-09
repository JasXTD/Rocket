mod args;
mod advanced;
mod bisect;
mod commits;
mod composed;
mod hosting;
mod conflicts;
mod interrupt;
mod lfs;
mod maintenance;
mod output;
mod reads;
mod review;
mod run;
mod selection;
mod ssh;

use args::{Command, Options};
use output::{Outcome, Problem};
use rocket_git::{CancellationToken, DiagnosticLog};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

fn log_directory(options: &Options) -> io::Result<PathBuf> {
    let path = options
        .log_dir
        .clone()
        .or_else(|| std::env::var_os("ROCKET_LOG_DIR").map(PathBuf::from));
    let path = match path {
        Some(path) => path,
        None => {
            #[cfg(windows)]
            {
                std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Rocket/logs/cli"))
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "log directory unavailable"))?
            }
            #[cfg(not(windows))]
            {
                rocket_git::local_storage::directory()?.join("logs").join("cli")
            }
        }
    };
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log directory must be absolute",
        ));
    }
    #[cfg(unix)]
    rocket_git::local_storage::create_protected(&path)?;
    Ok(path)
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let json = args::wants_json(&args);
    let options = match args::parse(args) {
        Ok(options) => options,
        Err(message) => {
            return deliver(
                Outcome::failure(Problem::usage(message)),
                "usage",
                Value::Null,
                json,
            );
        }
    };
    let command = options.command.label();
    if matches!(options.command, Command::Help | Command::Version) {
        let data = if matches!(options.command, Command::Help) {
            json!({"usage":args::HELP})
        } else {
            json!({"version":env!("CARGO_PKG_VERSION"),"json_schema_version":1})
        };
        return deliver(Outcome::success(data), command, Value::Null, options.json);
    }
    let hosting_token = match hosting::take_credential(&options) {
        Ok(token) => token,
        Err(error) => return deliver(Outcome::failure(Problem::backend(&error)), command, Value::Null, options.json),
    };
    if let Err(error) = rocket_git::ensure_unprivileged_process() {
        return deliver(Outcome::failure(Problem::backend(&error)), command, Value::Null, options.json);
    }
    let mut warnings = vec![];
    let crash_directory=options.log_dir.as_ref().map(|p|Ok(p.join("crashes"))).unwrap_or_else(rocket_git::crash_reports::default_directory);
    if rocket_git::crash_reports::install(crash_directory,rocket_git::crash_reports::Component::Cli).is_err(){
        warnings.push(json!({"code":"crash_capture_unavailable","message":"Local crash reporting could not initialize; the requested operation is unaffected."}));
    }
    let log = if options.no_log || std::env::var_os("ROCKET_LOG").is_some_and(|v| v == "off") {
        None
    } else {
        match log_directory(&options).and_then(DiagnosticLog::open) {
            Ok(log) => Some(log),
            Err(error) => {
                warnings.push(json!({"code":"diagnostics_unavailable","message":"Diagnostic logging could not start. Command results are unaffected.","os_error":error.raw_os_error()}));
                None
            }
        }
    };
    let cancel = CancellationToken::new();
    let mut outcome = match interrupt::Guard::install(cancel.clone(), options.cancel_after_ms) {
        Ok(guard) => {
            let result = match &log {
                Some(log) => log.request(command, |context| {
                    run::execute(&options, Some(context), &cancel, hosting_token.as_deref())
                }),
                None => run::execute(&options, None, &cancel, hosting_token.as_deref()),
            };
            // The backend has reaped its child/helper processes and delivered all
            // write observations. Join the watcher before printing one final result.
            drop(guard);
            match result {
                Ok(report) => report,
                Err(error) => Outcome::failure(Problem::backend(&error)),
            }
        }
        Err(error) => Outcome::failure(Problem::backend(&error)),
    };
    outcome.warnings.extend(warnings);
    let diagnostics=log.as_ref().map_or(Value::Null,|log| {
        let health=log.health();
        if health.dropped_events>0 {outcome.warnings.push(json!({"code":"diagnostics_dropped","dropped_events":health.dropped_events,"message":"Some diagnostics could not be saved; command outcome is unchanged."}));}
        json!({"path":output::os_path(&log.directory().join("rocket.jsonl")),"session_id":log.session_id(),"dropped_events":health.dropped_events})
    });
    deliver(outcome, command, diagnostics, options.json)
}

fn deliver(outcome: Outcome, command: &str, diagnostics: Value, json: bool) -> ExitCode {
    let exit = outcome.exit();
    let delivered = if !json && exit != 0 {
        let mut stderr = io::stderr().lock();
        output::render(&mut stderr, &outcome, command, diagnostics, false)
            .and_then(|_| stderr.flush())
    } else {
        let mut stdout = io::stdout().lock();
        output::render(&mut stdout, &outcome, command, diagnostics, json)
            .and_then(|_| stdout.flush())
    };
    if let Err(error) = delivered {
        // Never panic or retry a completed write because the consumer closed its
        // output pipe. The command may have succeeded even though delivery failed.
        let _ = writeln!(
            io::stderr(),
            "Result output could not be written ({:?}); inspect repository state before retrying.",
            error.kind()
        );
        return ExitCode::from(if exit == 0 { 1 } else { exit });
    }
    ExitCode::from(exit)
}
