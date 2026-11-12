//! Review and revalidation stay off the UI thread. Worktree writes reuse Git's
//! existing write permits, stale checks and recovery reports.
use super::model::{Command, Record, invalid};
use crate::backend::{Backend, user_error};
use rocket_git::*;
use std::sync::Arc;
mod program;
pub use program::Program;
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Create,
    Restart,
    Resume,
    Remove,
}
pub struct Plan {
    pub record: Record,
    pub mode: Mode,
    pub worktree: Option<WorktreePreview>,
    pub target: ObjectId,
    pub program: Option<Arc<Program>>,
    pub setup: Option<Arc<Program>>,
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
            Mode::Restart | Mode::Resume => None,
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
                Some(Arc::new(Program::inspect(&record.command, cancel)?)),
                if mode == Mode::Resume {
                    None
                } else {
                    record.setup.as_ref()
                        .map(|c| Program::inspect(c, cancel).map(Arc::new))
                        .transpose()?
                },
            )
        };
        if mode == Mode::Resume {
            if !record.tracking || !super::events::can_resume(&record) || record.conversation.is_none() {
                return Err(invalid("This session needs a reviewed restart. One-click resume requires an exact saved conversation and recognized interactive arguments, without an initial prompt or exec/print command."));
            }
            let expected = approval(&record, &program.as_ref().unwrap().digest, cancel)?;
            if record.approval.as_deref()!=Some(&expected) {
                return Err(invalid("The executable or launch settings changed. Close this check and review a restart before running it."));
            }
        }
        if record.tracking && record.provider != super::model::Provider::Custom {
            let bad = record.command.args().iter().any(|a| {
                let flag=a.split('=').next().unwrap_or("");
                matches!(flag,"resume"|"--resume"|"-r"|"--continue"|"--session-id") || flag=="-c"&&record.provider!=super::model::Provider::Codex
            });
            if bad {return Err(invalid("Let Rocket manage resume arguments when conversation tracking is enabled."));}
        }
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
    pub bridge: Option<super::events::Bridge>,
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
    start_observed(backend, plan, after_setup, wake, cancel, || {})
}
// Deterministic security regression seam after all review checks and immediately
// before handing the executable to the asynchronous terminal launcher.
fn start_observed(
    backend: &Backend,
    plan: &Plan,
    after_setup: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
    cancel: &CancellationToken,
    before_launch: impl FnOnce(),
) -> Result<Started> {
    let operation = if plan.mode == Mode::Remove {
        "agent-session-remove"
    } else {
        "agent-session-start"
    };
    backend.request(operation, false, |git| {
        cancel.check()?;
        if let Some(program) = &plan.program {
            program.recheck(cancel)?;
        }
        if plan.mode != Mode::Resume && !after_setup && let Some(program) = &plan.setup {
            program.recheck(cancel)?;
        }
        let repo = git.open(&plan.record.source, cancel)?;
        if !after_setup && let Some(p) = &plan.worktree {
            let report = repo.apply_worktree(p, cancel)?;
            if let Err(error) = report.command {
                return Err(invalid(&format!("{} Worktree files are retained if creation began. Inspect Worktrees before retrying.", user_error(&error))));
            }
            report.verification?;
            report.observed?;
        }
        if plan.mode == Mode::Remove {
            return Ok(Started { session: None, setup: false, removed: true, bridge:None });
        }
        let e = entry(&repo, &plan.record, cancel)?;
        if !after_setup && e.head.as_ref() != Some(&plan.target) {
            return Err(invalid("The worktree commit changed after review. Review again."));
        }
        let setup = plan.mode != Mode::Resume && !after_setup && plan.setup.is_some();
        let (program, command) = if setup {
            (plan.setup.as_ref().unwrap(), plan.record.setup.as_ref().unwrap())
        } else {
            (plan.program.as_ref().unwrap(), &plan.record.command)
        };
        cancel.check()?;
        before_launch();
        cancel.check()?;
        let mut bridge = if !setup && plan.record.tracking {Some(super::events::Bridge::new(wake.clone()).map_err(|e|io("connect agent activity",e))?)}else{None};
        let arguments = if let Some(b)=&mut bridge {b.arguments(&plan.record,plan.mode==Mode::Resume).map_err(|e|io("prepare agent activity",e))?}else{command.args()};
        let environment=bridge.as_ref().map_or_else(Vec::new,|b|b.environment.clone());
        #[cfg(unix)]
        program.check_launch_policy()?;
        let session = rocket_terminal::Session::start_shell_held_with_environment(
            &plan.record.destination, 100, 18, wake,
            &rocket_terminal::Shell::Custom { program: program.path.clone(), arguments },
            program.launch_path(), program.clone(), environment,
        ).map_err(|e| io("start agent terminal", e))?;
        if cancel.is_cancelled() { session.stop(); }
        Ok(Started { session: Some(session), setup, removed: false, bridge })
    })
}

/// Binds resume to executable bytes and the reviewed launch context. Resume never runs setup.
pub(super) fn approval(r:&Record, digest:&str, cancel:&CancellationToken)->Result<String> {
    let bytes=serde_json::to_vec(&serde_json::json!([r.source,r.destination,r.branch,r.provider.key(),r.command.program,r.command.arguments,r.tracking,digest])).map_err(|e|invalid(&e.to_string()))?;
    sha256_reader(&mut std::io::Cursor::new(bytes),64*1024,cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::fs::OpenOptions;
    #[cfg(unix)]
    fn finished(session: &rocket_terminal::Session) -> String {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.status().closed && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.status().closed, "fixture process did not finish");
        assert!(session.status().error.is_none(), "{:?}", session.status());
        assert_eq!(session.status().exit_code, Some(0));
        session.snapshot(0).unwrap().text()
    }
    #[cfg(unix)]
    #[test]
    fn policy_revoked_during_worktree_preparation_prevents_terminal_start() {
        crate::test_common::isolated(
            "ROCKET_AGENT_POLICY_REVOCATION",
            "agents::runtime::tests::policy_revoked_during_worktree_preparation_prevents_terminal_start",
            || {
                use std::{fs, os::unix::fs::PermissionsExt};
                let fixture = crate::test_common::Fixture::new();
                fs::write(fixture.repository.join("base"), b"base\n").unwrap();
                fixture.commit("base");
                let executable = fixture.root.join("agent 文");
                fs::copy("/bin/true", &executable).unwrap();
                fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
                let backend = Backend::discover(&fixture.root.join("logs")).unwrap();
                let record = Record {
                    id: 1,
                    tracking: false, approval: None, conversation: None,
                    attention: false, last_activity: Default::default(),
                    name: "policy fixture".into(),
                    source: fixture.repository.clone(),
                    destination: fixture.root.join("worktree 文"),
                    branch: "agents/policy".into(), base: "HEAD".into(),
                    provider: super::super::model::Provider::Custom,
                    command: Command { program: executable.to_str().unwrap().into(), arguments: String::new() },
                    setup: None,
                };
                let cancel = CancellationToken::new();
                let plan = preview(&backend, record, Mode::Create, &cancel).unwrap();
                let snapshot = plan.program.as_ref().unwrap().launch_path().to_owned();
                let result = start_observed(&backend, &plan, false, Arc::new(|| {}), &cancel, || {
                    fs::set_permissions(&executable, fs::Permissions::from_mode(0o600)).unwrap();
                });
                assert!(matches!(result, Err(Error::InvalidInput(ref reason)) if reason.contains("not executable")));
                assert!(plan.record.destination.join("base").is_file(), "prepared worktree must stay recoverable");
                drop(plan);
                assert!(!snapshot.exists());
                assert_eq!(fs::metadata(executable).unwrap().permissions().mode() & 0o777, 0o600);
            },
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_swap_after_recheck_never_runs_replacement() {
        crate::test_common::isolated(
            "ROCKET_AGENT_EXECUTABLE_SWAP",
            "agents::runtime::tests::executable_swap_after_recheck_never_runs_replacement",
            || {
                use std::{
                    fs,
                    os::unix::fs::PermissionsExt,
                    time::{Duration, Instant},
                };
                let fixture = crate::test_common::Fixture::new();
                fs::write(fixture.repository.join("base"), b"base\n").unwrap();
                fixture.commit("base");
                let original = fixture.root.join("agent 資料");
                let replacement = fixture.root.join("replacement");
                for (path, marker) in [
                    (&original, "REVIEWED_EXECUTABLE"),
                    (&replacement, "UNREVIEWED_REPLACEMENT"),
                ] {
                    fs::write(path, format!("#!/bin/sh\nprintf '%s\\n' '{marker}'\nIFS= read -r line\nprintf '%s\\n' \"$line\"\n")).unwrap();
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                }
                let backend = Backend::discover(&fixture.root.join("logs")).unwrap();
                let record = Record {
                    tracking:false, conversation:None, approval:None,attention:false,last_activity:Default::default(),
                    id: 1,
                    name: "swap fixture".into(),
                    source: fixture.repository.clone(),
                    destination: fixture.root.join("worktree 文"),
                    branch: "agents/swap".into(),
                    base: "HEAD".into(),
                    provider: super::super::model::Provider::Custom,
                    command: Command {
                        program: original.to_string_lossy().into_owned(),
                        arguments: String::new(),
                    },
                    setup: None,
                };
                let cancel = CancellationToken::new();
                let plan = preview(&backend, record, Mode::Create, &cancel).unwrap();
                let snapshot = plan.program.as_ref().unwrap().launch_path().to_owned();
                let started =
                    start_observed(&backend, &plan, false, Arc::new(|| {}), &cancel, || {
                        fs::rename(&original, fixture.root.join("reviewed-inode")).unwrap();
                        fs::rename(&replacement, &original).unwrap();
                    })
                    .unwrap();
                let session = started.session.unwrap();
                drop(plan);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !session.status().running
                    && !session.status().closed
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    snapshot.exists(),
                    "dropping the review released the executable before the process ended"
                );
                session.input("finished 文\n".as_bytes()).unwrap();
                let output = finished(&session);
                assert!(
                    output.contains("REVIEWED_EXECUTABLE"),
                    "reviewed bytes did not run: {output}"
                );
                assert!(
                    !output.contains("UNREVIEWED_REPLACEMENT"),
                    "replacement ran: {output}"
                );
                assert!(
                    !snapshot.exists(),
                    "finished session retained its private executable"
                );
                assert!(
                    fs::read_to_string(&original)
                        .unwrap()
                        .contains("UNREVIEWED_REPLACEMENT")
                );
            },
        );
    }

    #[cfg(unix)]
    #[test]
    fn setup_handoff_rechecks_original_but_launches_reviewed_snapshots() {
        crate::test_common::isolated(
            "ROCKET_AGENT_SETUP_BINDING",
            "agents::runtime::tests::setup_handoff_rechecks_original_but_launches_reviewed_snapshots",
            || {
                use std::{fs, os::unix::fs::PermissionsExt};
                let fixture = crate::test_common::Fixture::new();
                fs::write(fixture.repository.join("base"), b"base\n").unwrap();
                fixture.commit("base");
                let agent = fixture.root.join("agent");
                let setup = fixture.root.join("setup");
                let replacement = fixture.root.join("replacement");
                let script = |marker: &str| format!("#!/bin/sh\nprintf '%s\\n' '{marker}'\n");
                for (path, marker) in [
                    (&agent, "REVIEWED_AGENT"),
                    (&setup, "REVIEWED_SETUP"),
                    (&replacement, "UNREVIEWED_SETUP"),
                ] {
                    fs::write(path, script(marker)).unwrap();
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                }
                let command = |path: &std::path::Path| Command {
                    program: path.to_string_lossy().into_owned(),
                    arguments: String::new(),
                };
                let backend = Backend::discover(&fixture.root.join("logs")).unwrap();
                let cancel = CancellationToken::new();
                let plan = preview(
                    &backend,
                    Record {
                        tracking:false, conversation:None, approval:None,attention:false,last_activity:Default::default(),
                        id: 1,
                        name: "setup fixture".into(),
                        source: fixture.repository.clone(),
                        destination: fixture.root.join("worktree"),
                        branch: "agents/setup-binding".into(),
                        base: "HEAD".into(),
                        provider: super::super::model::Provider::Custom,
                        command: command(&agent),
                        setup: Some(command(&setup)),
                    },
                    Mode::Create,
                    &cancel,
                )
                .unwrap();
                let started =
                    start_observed(&backend, &plan, false, Arc::new(|| {}), &cancel, || {
                        fs::rename(&setup, fixture.root.join("old-setup")).unwrap();
                        fs::rename(&replacement, &setup).unwrap();
                    })
                    .unwrap();
                assert!(started.setup);
                let output = finished(&started.session.unwrap());
                assert!(
                    output.contains("REVIEWED_SETUP") && !output.contains("UNREVIEWED_SETUP"),
                    "{output}"
                );
                // Setup may intentionally install/update a tool. Any changed bytes
                // must force a fresh review instead of silently approving that update.
                fs::write(&agent, script("CHANGED_DURING_SETUP")).unwrap();
                assert!(start(&backend, &plan, true, Arc::new(|| {}), &cancel).is_err());
                fs::write(&agent, script("REVIEWED_AGENT")).unwrap();
                let interrupted = CancellationToken::new();
                let result =
                    start_observed(&backend, &plan, true, Arc::new(|| {}), &interrupted, || {
                        interrupted.cancel()
                    });
                assert!(matches!(result, Err(Error::Cancelled)));
                // Also bind content, not just an inode: overwriting the original
                // after recheck must not alter the private executable's bytes.
                let started =
                    start_observed(&backend, &plan, true, Arc::new(|| {}), &cancel, || {
                        fs::write(&agent, script("UNREVIEWED_AGENT")).unwrap();
                    })
                    .unwrap();
                assert!(!started.setup);
                let output = finished(&started.session.unwrap());
                assert!(
                    output.contains("REVIEWED_AGENT") && !output.contains("UNREVIEWED_AGENT"),
                    "{output}"
                );
            },
        );
    }
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
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
