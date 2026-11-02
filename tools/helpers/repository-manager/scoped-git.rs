// Owned native-test relay: hold only Git started in one disposable repository.
// Other open tabs can continue refreshing, so cancellation tests do not race
// with unrelated reads or depend on hooks (which the MCP intentionally disables).
use std::{env, fs, process::{Command, Stdio}, thread, time::Duration};

fn main() {
    let gate = env::var_os("ROCKET_FILE_TEST_GATE").expect("fixture gate");
    let marker = env::var_os("ROCKET_FILE_TEST_MARKER").expect("fixture marker");
    let requested = fs::read_to_string(&gate).unwrap_or_default();
    let here = env::current_dir().and_then(fs::canonicalize).expect("fixture directory");
    if !requested.is_empty() && fs::canonicalize(&requested).ok().as_ref() == Some(&here) {
        fs::write(marker, std::process::id().to_string()).expect("fixture marker");
        while fs::read_to_string(&gate).ok().as_deref() == Some(&requested) {
            thread::sleep(Duration::from_millis(10));
        }
    }
    let mut git = Command::new(env::var_os("ROCKET_FILE_TEST_GIT").expect("installed Git"));
    git.args(env::args_os().skip(1)).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        git.creation_flags(0x08000000);
    }
    std::process::exit(git.status().expect("real Git").code().unwrap_or(1));
}
