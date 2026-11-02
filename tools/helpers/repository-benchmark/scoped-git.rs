// Only holds/fails the selected read in one disposable repository. All other
// requests, including MCP policy validation, continue through installed Git.
use std::{
    env, fs,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let gate = env::var_os("ROCKET_BENCHMARK_GATE").expect("fixture gate");
    let value = fs::read_to_string(&gate).unwrap_or_default();
    let lines: Vec<_> = value.lines().collect();
    if lines.len() == 3
        && fs::canonicalize(lines[0]).ok() == env::current_dir().and_then(fs::canonicalize).ok()
        && args.iter().any(|a| a == lines[1])
    {
        let marker = env::var_os("ROCKET_BENCHMARK_MARKER").expect("fixture marker");
        // Diff preparation can run patch and image-identity reads in parallel.
        // Separate PID filenames cannot be observed while another writer truncates them.
        let marker = std::path::PathBuf::from(marker).join(std::process::id().to_string());
        fs::write(marker, []).expect("marker");
        if lines[2] == "fail" {
            eprintln!("synthetic PRIVATE benchmark read failure");
            std::process::exit(1);
        }
        while fs::read_to_string(&gate).ok().as_deref() == Some(&value) {
            thread::sleep(Duration::from_millis(10));
        }
    }
    let mut git = Command::new(env::var_os("ROCKET_BENCHMARK_GIT").expect("installed Git"));
    git.args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        git.creation_flags(0x08000000);
    }
    std::process::exit(git.status().expect("Git child").code().unwrap_or(1));
}
