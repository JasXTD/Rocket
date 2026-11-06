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
    Remember,
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
                                // "Enter passphrase (empty …", or in newer OpenSSH
                                // (10.2 on Ubuntu) "Enter passphrase for "<key>" (empty …",
                                // which can wrap on a long path: match where it starts.
                                "Enter passphrase",
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

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::{process::Command, sync::mpsc};

    const NAME: &str = "ssh_keys::console::linux_tests::linux_openssh_prompts_create_and_unlock_a_key";
    const PHRASE: &str = "disposable console phrase";

    /// Runs `tool` in the console, answering each expected prompt with PHRASE.
    fn run(root: &Path, tool: &'static str, args: Vec<String>, expected: Vec<Prompt>) -> Option<u32> {
        let (input, receive) = mpsc::sync_channel(1);
        let (prompts, prompt_receive) = mpsc::sync_channel(4);
        let (directory, log) = (root.to_path_buf(), DiagnosticLog::open(root.join("logs")).unwrap());
        let worker = thread::spawn(move || {
            console(&directory, tool, args, receive, prompts, &CancellationToken::new(), &log)
        });
        for wanted in expected {
            assert_eq!(prompt_receive.recv_timeout(Duration::from_secs(15)), Ok(wanted), "{tool}");
            input.send(PHRASE.to_owned()).unwrap();
        }
        worker.join().unwrap().unwrap()
    }

    /// The system's OpenSSH on a Linux pseudoterminal: the prompts of a new key and of
    /// unlocking it are recognised, and the passphrase only goes to them.
    #[test]
    fn linux_openssh_prompts_create_and_unlock_a_key() {
        crate::test_common::isolated("ROCKET_SSH_CONSOLE_CASE", NAME, || {
            // The second run, below: the console's ssh-add finds the agent SSH_AUTH_SOCK names.
            if let Some(key) = std::env::var_os("ROCKET_SSH_CONSOLE_KEY").map(PathBuf::from) {
                let unlocked = run(key.parent().unwrap(), "ssh-add.exe", vec!["-k".into(), key.display().to_string()], vec![Prompt::Unlock]);
                assert_eq!(unlocked, Some(0));
                return;
            }
            {
                let f = crate::test_common::Fixture::new();
                let root = std::fs::canonicalize(&f.root).unwrap();
                let key = root.join("id_ed25519");
                let created = run(
                    &root,
                    "ssh-keygen.exe",
                    ["-t", "ed25519", "-a", "16", "-C", "Rocket", "-f"].map(String::from).into_iter().chain([key.display().to_string()]).collect(),
                    vec![Prompt::New, Prompt::Confirm],
                );
                assert_eq!(created, Some(0));
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
                // Encrypted: an empty passphrase cannot read it.
                let check = Command::new("/usr/bin/ssh-keygen").args(["-y", "-P", "", "-f"]).arg(&key).output().unwrap();
                assert!(!check.status.success());

                // Unlock it into a private agent: this test runs again with SSH_AUTH_SOCK
                // naming it (the isolated run clears SSH_ variables).
                let socket = root.join("agent.sock");
                let mut agent = Command::new("/usr/bin/ssh-agent").arg("-D").arg("-a").arg(&socket).spawn().unwrap();
                let start = Instant::now();
                while !socket.exists() {
                    assert!(start.elapsed() < Duration::from_secs(10), "ssh-agent did not start");
                    thread::sleep(Duration::from_millis(20));
                }
                let again = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", NAME, "--nocapture", "--include-ignored"])
                    .env("SSH_AUTH_SOCK", &socket)
                    .env("ROCKET_SSH_CONSOLE_KEY", &key)
                    .output()
                    .unwrap();
                assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stdout));
                let listed = Command::new("/usr/bin/ssh-add").arg("-l").env("SSH_AUTH_SOCK", &socket).output().unwrap();
                assert!(String::from_utf8_lossy(&listed.stdout).contains("Rocket (ED25519)"), "{listed:?}");
                agent.kill().unwrap();
                agent.wait().unwrap();
            }
        });
    }
}
