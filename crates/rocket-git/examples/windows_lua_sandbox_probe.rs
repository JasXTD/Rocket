//! Test-only raw OS probes through the same MSIX bridge as the shipped helper.
//! This executable is never included in a Rocket package.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[allow(unused_attributes)]
#[path = "../src/integration_worker/windows/probe.rs"]
mod probe;

#[cfg(windows)]
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--msix-lua-bridge") {
        let result = (args.len() == 3)
            .then(|| {
                args[1]
                    .to_str()
                    .zip(args[2].to_str()?.parse::<usize>().ok())
            })
            .flatten()
            .ok_or_else(|| "Invalid bridge arguments".to_owned())
            .and_then(|(nonce, cancel_event)| {
                rocket_git::IntegrationWorker::run_msix_bridge(nonce, cancel_event)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    } else {
        probe::main();
    }
}

#[cfg(not(windows))]
fn main() {}
