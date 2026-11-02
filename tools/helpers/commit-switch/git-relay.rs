// Disposable test-only Git relay. Installed only on the test child's PATH.
use std::{env,fs,process::{Command,Stdio},thread,time::Duration};
fn main(){
    let args:Vec<_>=env::args_os().skip(1).collect();
    let gate=env::var_os("ROCKET_FILE_TEST_GATE").unwrap();
    let marker=env::var_os("ROCKET_FILE_TEST_MARKER").unwrap();
    let mode=fs::read_to_string(&gate).unwrap_or_default();
    let part=mode.split_once(':');
    let matched=part.is_some_and(|(_,cmd)|args.iter().any(|a|a==cmd));
    let pause=||{fs::write(&marker,std::process::id().to_string()).unwrap();while fs::read_to_string(&gate).ok().as_deref()==Some(&mode){thread::sleep(Duration::from_millis(10));}};
    if matched&&part.unwrap().0=="before" {pause();}
    let mut cmd=Command::new(env::var_os("ROCKET_FILE_TEST_GIT").unwrap());
    cmd.args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    #[cfg(windows)]{use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
    let status=cmd.status().expect("real Git");
    if matched&&part.unwrap().0=="after" {pause();}
    std::process::exit(status.code().unwrap_or(1));
}
