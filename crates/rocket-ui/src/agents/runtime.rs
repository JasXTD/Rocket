//! Review and revalidation stay off the UI thread. Worktree writes reuse Git's
//! existing write permits, stale checks and recovery reports.
use super::model::{Command, Record, invalid};
use crate::backend::{Backend, user_error};
use rocket_git::*;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Create,
    Restart,
    Remove,
}
pub struct Program {
    pub path: PathBuf,
    pub digest: String,
    // On Windows deny replacement/writes for the whole review and launch. Unix
    // executable paths are rehashed at launch; these sessions are not a sandbox.
    _file: File,
}
impl Program {
    fn inspect(command: &Command, cancel: &CancellationToken) -> Result<Self> {
        command.validate()?;
        let path = Path::new(&command.program);
        let meta =
            std::fs::symlink_metadata(path).map_err(|e| io("inspect agent executable", e))?;
        #[cfg(windows)]
        let redirected = {
            use std::os::windows::fs::MetadataExt;
            meta.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let redirected = meta.file_type().is_symlink();
        if !meta.is_file() || redirected {
            return Err(invalid(
                "Choose a regular executable, not a link or directory.",
            ));
        }
        let path = path
            .canonicalize()
            .map_err(|e| io("resolve agent executable", e))?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let file = options
            .open(&path)
            .map_err(|e| io("hold reviewed agent executable", e))?;
        let digest = osv_file_digest(&path, 512 * 1024 * 1024, cancel)?;
        Ok(Self {
            path,
            digest,
            _file: file,
        })
    }
    fn recheck(&self, cancel: &CancellationToken) -> Result<()> {
        if osv_file_digest(&self.path, 512 * 1024 * 1024, cancel)? != self.digest {
            return Err(invalid(
                "Executable changed after review. Review the session again.",
            ));
        }
        Ok(())
    }
}
pub struct Plan {
    pub record: Record,
    pub mode: Mode,
    pub worktree: Option<WorktreePreview>,
    pub target: ObjectId,
    pub program: Option<Program>,
    pub setup: Option<Program>,
}
pub(super) fn entry(
    repo: &Repository,
    record: &Record,
    cancel: &CancellationToken,
) -> Result<WorktreeEntry> {
    let destination = record
        .destination
        .canonicalize()
        .map_err(|e| io("find agent worktree", e))?;
    let expected = format!("refs/heads/{}", record.branch);
    let entry = repo
        .worktrees(cancel)?
        .into_iter()
        .find(|e| e.path.canonicalize().ok().as_ref() == Some(&destination))
        .ok_or_else(|| {
            invalid("The saved worktree is no longer registered with this repository.")
        })?;
    if entry.main || entry.branch.as_deref() != Some(expected.as_bytes()) || entry.locked.is_some()
    {
        return Err(invalid(
            "The worktree branch or lock changed. Review it in Worktrees before continuing.",
        ));
    }
    // Prove the directory still belongs to this repository, not a replacement.
    repo.open_worktree(&entry, cancel)?;
    Ok(entry)
}
pub fn preview(
    backend: &Backend,
    mut record: Record,
    mode: Mode,
    cancel: &CancellationToken,
) -> Result<Plan> {
    record.validate()?;
    backend.request("agent-session-preview", false, |git| {
        let repo = git.open(&record.source, cancel)?;
        record.source = repo
            .info()
            .worktree
            .clone()
            .ok_or(Error::WorktreeRequired)?;
        let worktree = match mode {
            Mode::Create => Some(repo.preview_create_worktree(
                &record.destination,
                WorktreeCheckout::NewBranch(record.branch.clone()),
                &record.base,
                cancel,
            )?),
            Mode::Remove => {
                Some(repo.preview_remove_worktree(&entry(&repo, &record, cancel)?, cancel)?)
            }
            Mode::Restart => None,
        };
        let target = if let Some(p) = &worktree {
            record.destination = p.path().to_owned();
            p.target().clone()
        } else {
            entry(&repo, &record, cancel)?
                .head
                .ok_or_else(|| invalid("The agent worktree has no commit."))?
        };
        let (program, setup) = if mode == Mode::Remove {
            (None, None)
        } else {
            (
                Some(Program::inspect(&record.command, cancel)?),
                record
                    .setup
                    .as_ref()
                    .map(|c| Program::inspect(c, cancel))
                    .transpose()?,
            )
        };
        Ok(Plan {
            record,
            mode,
            worktree,
            target,
            program,
            setup,
        })
    })
}
pub struct Started {
    pub session: Option<rocket_terminal::Session>,
    pub setup: bool,
    pub removed: bool,
}
pub fn start(
    backend: &Backend,
    plan: &Plan,
    after_setup: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
    cancel: &CancellationToken,
) -> Result<Started> {
    backend.request(if plan.mode==Mode::Remove{"agent-session-remove"}else{"agent-session-start"},false,|git| {
        cancel.check()?;
        if let Some(program)=&plan.program {program.recheck(cancel)?;}
        if !after_setup && let Some(program)=&plan.setup {program.recheck(cancel)?;}
        let repo=git.open(&plan.record.source,cancel)?;
        if !after_setup && let Some(p)=&plan.worktree {
            let report=repo.apply_worktree(p,cancel)?;
            if let Err(error)=report.command {return Err(invalid(&format!("{} Worktree files are retained if creation began. Inspect Worktrees before retrying.",user_error(&error))));}
            report.verification?;
            report.observed?;
        }
        if plan.mode==Mode::Remove {return Ok(Started{session:None,setup:false,removed:true});}
        let e=entry(&repo,&plan.record,cancel)?;
        if !after_setup && e.head.as_ref()!=Some(&plan.target) {return Err(invalid("The worktree commit changed after review. Review again."));}
        let setup=!after_setup && plan.setup.is_some();
        let (program,command)=if setup {(plan.setup.as_ref().unwrap(),plan.record.setup.as_ref().unwrap())}else{(plan.program.as_ref().unwrap(),&plan.record.command)};
        cancel.check()?;
        let session=rocket_terminal::Session::start_shell(&plan.record.destination,100,18,wake,&rocket_terminal::Shell::Custom{program:program.path.clone(),arguments:command.args()}).map_err(|e|io("start agent terminal",e))?;
        if cancel.is_cancelled() {session.stop();}
        Ok(Started{session:Some(session),setup,removed:false})
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_review_binds_executable_and_cancellation_never_approves_it() {
        let root = std::env::temp_dir().join(format!(
            "rocket-agent-program-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("fixture.exe");
        // Non-executable fixture bytes: this test reviews a file, never runs it.
        std::fs::write(&path, b"first version").unwrap();
        let command = Command {
            program: path.to_string_lossy().into_owned(),
            arguments: String::new(),
        };
        let cancel = CancellationToken::new();
        let program = Program::inspect(&command, &cancel).unwrap();
        program.recheck(&cancel).unwrap();
        #[cfg(windows)]
        {
            assert!(OpenOptions::new().write(true).open(&path).is_err());
            assert!(std::fs::rename(&path, root.join("replacement.exe")).is_err());
        }
        #[cfg(not(windows))]
        {
            std::fs::write(&path, b"second version").unwrap();
            assert!(program.recheck(&cancel).is_err());
        }
        cancel.cancel();
        assert!(program.recheck(&cancel).is_err());
        assert!(Program::inspect(&command, &cancel).is_err());
        drop(program);
        std::fs::write(&path, b"unlocked after review").unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
