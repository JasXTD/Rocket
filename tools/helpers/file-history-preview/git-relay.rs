// Acceptance-only relay: all commands execute the real installed Git. A gate
// pauses draining selected read output so cancellation can be exercised reliably.
// Never installed on the user's PATH; only the disposable MCP child inherits it.
use std::{env,fs,io,process::{Command,Stdio},thread,time::Duration};
fn main() {
    let args:Vec<_>=env::args_os().skip(1).collect();
    let gate=env::var_os("ROCKET_FILE_TEST_GATE").expect("fixture gate");
    let marker=env::var_os("ROCKET_FILE_TEST_MARKER").expect("fixture marker");
    let mode=fs::read_to_string(&gate).unwrap_or_default();
    let pause=(!mode.is_empty()) && args.iter().any(|a|a==mode.as_str()) && (mode!="log" || args.iter().any(|a|a=="--follow"));
    let mut command=Command::new(env::var_os("ROCKET_FILE_TEST_GIT").expect("real Git"));
    command.args(args).stdin(Stdio::inherit()).stderr(Stdio::inherit()).stdout(if pause {Stdio::piped()} else {Stdio::inherit()});
    #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    let mut child=command.spawn().expect("start real Git");
    if pause {
        fs::write(marker,format!("{}\n{}",std::process::id(),child.id())).expect("fixture marker");
        while fs::read_to_string(&gate).ok().as_deref()==Some(mode.as_str()) {thread::sleep(Duration::from_millis(10));}
        io::copy(&mut child.stdout.take().expect("pipe"),&mut io::stdout()).expect("forward real Git output");
    }
    std::process::exit(child.wait().expect("wait for Git").code().unwrap_or(1));
}
