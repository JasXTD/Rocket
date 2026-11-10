//! Fixed local acceptance process, not a general command runner.
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};
pub fn run(mode: &str) -> Result<(), String> {
    if !["hold", "setup", "fail", "child"].contains(&mode) {
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
