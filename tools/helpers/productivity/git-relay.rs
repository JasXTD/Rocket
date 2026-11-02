// Owned test helper only. Forward to the installed Git; deterministically pause
// before launch or after Git completes so cancellation never relies on a race.
use std::{env, fs, process::{Command, Stdio}, thread, time::Duration};
fn hold(gate: &std::path::Path, marker: &std::path::Path, mode: &str) {
    fs::write(marker, std::process::id().to_string()).expect("fixture marker");
    while fs::read_to_string(gate).ok().as_deref() == Some(mode) {
        thread::sleep(Duration::from_millis(10));
    }
}
fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let gate = std::path::PathBuf::from(env::var_os("ROCKET_FILE_TEST_GATE").expect("fixture gate"));
    let marker = std::path::PathBuf::from(env::var_os("ROCKET_FILE_TEST_MARKER").expect("fixture marker"));
    let mode = fs::read_to_string(&gate).unwrap_or_default();
    let point = mode.split_once(':').filter(|(_, name)| args.iter().any(|a| a == *name));
    if point.is_some_and(|(when, _)| when == "before") { hold(&gate, &marker, &mode); }
    let mut command = Command::new(env::var_os("ROCKET_FILE_TEST_GIT").expect("installed Git"));
    command.args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let mut child = command.spawn().expect("real Git");
    // A runner that held this command can still check that Stop ends the real Git too.
    if point.is_some_and(|(when, _)| when == "before") { fs::write(marker.with_extension("child"), child.id().to_string()).expect("fixture marker"); }
    let status = child.wait().expect("real Git");
    if point.is_some_and(|(when, _)| when == "after") { hold(&gate, &marker, &mode); }
    std::process::exit(status.code().unwrap_or(1));
}
