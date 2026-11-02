//! The passphrase travels only through an owned OpenSSH console.
use super::*;
use rocket_terminal::Session;
use std::{
    sync::mpsc::{Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    New,
    Confirm,
    Unlock,
    NewRetry,
    UnlockRetry,
}
pub(super) fn console(
    directory: &Path,
    tool: &str,
    args: Vec<String>,
    input: Receiver<String>,
    prompts: SyncSender<Prompt>,
    cancel: &CancellationToken,
    log: &DiagnosticLog,
) -> Result<Option<u32>> {
    let generation = tool == "ssh-keygen.exe";
    let session =
        Session::start_openssh(directory, tool, args, Arc::new(|| {})).map_err(|source| {
            Error::Io {
                action: "start private OpenSSH console",
                source,
            }
        })?;
    let start = Instant::now();
    let mut stopping = None;
    let mut revision = 0;
    let mut last_prompt = String::new();
    let mut waiting = false;
    loop {
        let status = session.status();
        if status.closed {
            log.ssh_console_exit(
                status.exit_code,
                start.elapsed(),
                stopping.map(|t: Instant| t.elapsed()),
            );
            if status.error.is_some() {
                return Err(Error::Ssh(SshFailure::ToolFailed));
            }
            return Ok(status.exit_code);
        }
        if (cancel.is_cancelled() || start.elapsed() > Duration::from_secs(180))
            && stopping.is_none()
        {
            session.stop();
            stopping = Some(Instant::now());
            log.request("ssh-stop", |_| Ok(()))?;
        }
        if let Some(stop) = stopping {
            if stop.elapsed() > Duration::from_secs(15) {
                return Err(Error::Ssh(SshFailure::RecoveryRequired));
            }
        } else {
            if let Some(screen) = session.snapshot(revision) {
                revision = screen.revision;
                let text = screen.text();
                // OpenSSH writes prompts without a trailing newline. Only enable input
                // after a known masked prompt, never during startup/echo-enabled input.
                let tail = text.trim_end();
                let prompt = if tail.ends_with(':') {
                    let mut candidates = vec![];
                    for (needle, prompt) in if generation {
                        vec![
                            (
                                "Enter passphrase (",
                                if tail.contains("Passphrases do not match") {
                                    Prompt::NewRetry
                                } else {
                                    Prompt::New
                                },
                            ),
                            ("Enter same passphrase again:", Prompt::Confirm),
                        ]
                    } else {
                        vec![
                            ("Enter passphrase for", Prompt::Unlock),
                            ("Bad passphrase, try again", Prompt::UnlockRetry),
                        ]
                    } {
                        if let Some(at) = tail.rfind(needle) {
                            candidates.push((at, prompt));
                        }
                    }
                    candidates.into_iter().max_by_key(|v| v.0).map(|v| v.1)
                } else {
                    None
                };
                if let Some(prompt) = prompt
                    && tail != last_prompt
                {
                    last_prompt = tail.to_owned();
                    waiting = true;
                    let _ = prompts.try_send(prompt);
                    log.request("ssh-prompt", |_| Ok(()))?;
                }
            }
            if waiting && let Ok(secret) = input.try_recv() {
                let valid = secret.len() <= 1024
                    && !secret.chars().any(char::is_control)
                    && (!generation || secret.chars().count() >= 10);
                let mut bytes = secret.into_bytes();
                if valid {
                    bytes.push(b'\r');
                    session.input(&bytes).map_err(|source| Error::Io {
                        action: "send private OpenSSH input",
                        source,
                    })?;
                    waiting = false;
                }
                bytes.fill(0);
                if !valid {
                    session.stop();
                    stopping = Some(Instant::now());
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}
