use rocket_git::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    process::{Command, Stdio},
};
pub fn path(value: &OsStr) -> Result<RepoPath> {
    let text = value
        .to_str()
        .ok_or_else(|| Error::InvalidInput("Review paths must be valid Unicode.".into()))?;
    RepoPath::new(text.replace('\\', "/").into_bytes())
}
pub fn launch(
    repo: &RepositoryInfo,
    id: &ObjectId,
    path: Option<&RepoPath>,
    blame: bool,
) -> Result<u32> {
    ensure_unprivileged_process()?;
    let executable = std::env::current_exe().map_err(|source| Error::Io {
        action: "locate Rocket CLI",
        source,
    })?;
    let desktop = executable
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("rocket.exe");
    if !desktop.is_file() {
        return Err(Error::InvalidInput("Native reviews require rocket.exe beside rocket-cli.exe. Use the complete Rocket bundle.".into()));
    }
    let root: PathBuf = repo
        .worktree
        .clone()
        .unwrap_or_else(|| repo.git_dir.clone());
    let mut cmd = Command::new(desktop);
    // The GUI outlives this launcher. Keeping a redirected CLI pipe open would
    // make JSON consumers wait for the desktop process to exit.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.arg("--review")
        .arg(if blame {
            "blame"
        } else if path.is_some() {
            "file"
        } else {
            "commit"
        })
        .arg("--repo")
        .arg(root)
        .arg("--commit")
        .arg(id.as_str());
    if let Some(path) = path {
        cmd.arg("--path").arg(path.display_lossy().as_ref());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn()
        .map(|child| child.id())
        .map_err(|source| Error::Io {
            action: "open native review window",
            source,
        })
}
