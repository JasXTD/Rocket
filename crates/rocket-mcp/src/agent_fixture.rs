//! Fixed local acceptance process, not a general command runner.
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};
pub fn run(mode: &str) -> Result<(), String> {
    run_inner(mode,None)
}
/// Only the fixed lifecycle probe tolerates generated provider hook/resume argv.
/// They are inspected, never executed or interpreted as commands/configuration.
pub fn lifecycle_launch(args:&[std::ffi::OsString])->Option<Result<(),String>> {
    let mut values:Vec<_>=args.iter().map(|a|a.to_string_lossy().into_owned()).collect();
    let mut resume=None;
    if values.first().is_some_and(|a|a=="resume") && values.len()>=2 {
        resume=Some(values.remove(1));values.remove(0);
    }
    if values.get(0).map(String::as_str)!=Some("-c") || values.get(1).map(String::as_str)!=Some("rocket_agent_fixture=lifecycle") {return None;}
    for pair in values[2..].chunks(2) {
        if pair.len()!=2{return Some(Err("Incomplete lifecycle probe arguments".into()));}
        match pair[0].as_str() {
            "-c" if pair[1].starts_with("hooks.")&&pair[1].len()<8192=>{},
            "--resume"=>resume=Some(pair[1].clone()),
            "--settings" if pair[1].len()<32768=>{},
            "--plugin-dir" if std::path::Path::new(&pair[1]).is_absolute()=>{},
            _=>return Some(Err("Unrecognized lifecycle probe argument".into())),
        }
    }
    if resume.as_deref().is_some_and(|s|!rocket_ui::agents::events::valid_id(s)){return Some(Err("Invalid conversation identity".into()));}
    Some(run_inner("lifecycle",resume))
}
fn notify(kind:&str,id:&str)->Result<(),String> {
    let mut command=std::process::Command::new(std::env::current_exe().map_err(|e|e.to_string())?);
    command.args(["--rocket-agent-event",kind]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x0800_0000);}
    let mut child=command.spawn().map_err(|e|e.to_string())?;
    let bytes=serde_json::to_vec(&serde_json::json!({"session_id":id,"tool_name":"synthetic","prompt":"PRIVATE_LIFECYCLE_PAYLOAD","last_assistant_message":"PRIVATE_LIFECYCLE_RESPONSE"})).unwrap();
    child.stdin.take().unwrap().write_all(&bytes).map_err(|e|e.to_string())?;
    if !child.wait().map_err(|e|e.to_string())?.success(){return Err("Lifecycle callback failed".into());}
    Ok(())
}
fn run_inner(mode:&str,resume:Option<String>)->Result<(),String> {
    if !["hold", "setup", "fail", "child", "lifecycle"].contains(&mode) {
        return Err("Unknown agent fixture mode".into());
    }
    let root = PathBuf::from(
        std::env::var_os("ROCKET_AGENT_FIXTURE_ROOT").ok_or("Agent fixture root is required")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if cwd == root || !cwd.starts_with(&root) || !cwd.join(".git").is_file() {
        return Err("Agent probes require an owned linked worktree".into());
    }
    if mode == "fail" {
        return Err("Synthetic setup failure".into());
    }
    if mode == "setup" {
        println!("SYNTHETIC_SETUP_OK 資料");
        return Ok(());
    }
    if mode == "child" {
        println!("SYNTHETIC_CHILD {}", std::process::id());
        io::stdout().flush().map_err(|e| e.to_string())?;
        loop {
            std::thread::park();
        }
    }
    let conversation=resume.unwrap_or_else(||{let s=rocket_mcp_control::ephemeral_token().unwrap();format!("{}-{}-{}-{}-{}",&s[..8],&s[8..12],&s[12..16],&s[16..20],&s[20..32])});
    if mode=="lifecycle" {notify("SessionStart",&conversation)?;}
    let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    child.args(["--agent-session-fixture", "child"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x0800_0000);
    }
    let mut child = child.spawn().map_err(|e| e.to_string())?;
    println!("SYNTHETIC_AGENT_READY 資料 child={}", child.id());
    io::stdout().flush().map_err(|e| e.to_string())?;
    for line in io::stdin().lock().lines() {
        match line.map_err(|e| e.to_string())?.trim() {
            "working" if mode=="lifecycle" => notify("UserPromptSubmit",&conversation)?,
            "attention" if mode=="lifecycle" => notify("PermissionRequest",&conversation)?,
            "complete" if mode=="lifecycle" => notify("Stop",&conversation)?,
            "interrupt" if mode=="lifecycle" => notify("Interrupt",&conversation)?,
            "status" => {
                println!("SYNTHETIC_AGENT_ALIVE");
                io::stdout().flush().map_err(|e| e.to_string())?;
            }
            "finish" => break,
            _ => println!("Only status and finish are accepted"),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
