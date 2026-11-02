// Owned local acceptance helper. Never used by Rocket's shipped Git discovery.
// A prepared:<folder> gate waits only after Rocket creates its staging folder.
use std::{env, fs, path::Path, process::{Command, Stdio}, thread, time::Duration};
fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let gate = env::var_os("ROCKET_FILE_TEST_GATE").unwrap();
    let marker = env::var_os("ROCKET_FILE_TEST_MARKER").unwrap();
    let mode = fs::read_to_string(&gate).unwrap_or_default();
    let ordinary = mode.split_once(':').filter(|(_, name)| args.iter().any(|a| a == *name));
    let prepared = mode.strip_prefix("prepared:").is_some_and(|parent| {
        args.iter().any(|a| a == "rev-parse") && fs::read_dir(Path::new(parent)).is_ok_and(|entries| entries.filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with(".rocket-patch-series-")))
    });
    let hold = || {
        fs::write(&marker, std::process::id().to_string()).unwrap();
        while fs::read_to_string(&gate).ok().as_deref() == Some(&mode) { thread::sleep(Duration::from_millis(10)); }
    };
    if prepared || ordinary.is_some_and(|(when,_)|when=="before") { hold(); }
    let mut command = Command::new(env::var_os("ROCKET_FILE_TEST_GIT").unwrap());
    command.args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let status = command.status().unwrap();
    if ordinary.is_some_and(|(when,_)|when=="after") { hold(); }
    std::process::exit(status.code().unwrap_or(1));
}
