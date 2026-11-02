//! Opt-in GitHub acceptance, never invoked by cargo test. No third-party code.
//! The relay sees encrypted bytes only; raw Git errors stay in process memory.
#[path = "support/clone_relay.rs"]
mod relay;

use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::ToSocketAddrs,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn save(path: &Path, value: &str) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(value.as_bytes())
}
fn git_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "/").replace('\'', "'\"'\"'"))
}
fn receiving(line: &str) -> bool {
    let Some(value) = line.strip_prefix("Clone Receiving: ") else {
        return false;
    };
    let Some((current, total)) = value.split_once(" / Some(") else {
        return false;
    };
    let current = current.parse::<u64>().unwrap_or(0);
    let total = total
        .split(')')
        .next()
        .unwrap_or("")
        .parse::<u64>()
        .unwrap_or(0);
    current > 0 && current < total
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    require(
        args.len() == 4 || args.len() == 6,
        "usage: clone_transport_probe <https|ssh> <cancel|disconnect|complete> <new-run-directory> <isolated-git-config> [<ssh-exe> <ssh-config>]",
    )?;
    let https = args[0] == "https";
    require(https || args[0] == "ssh", "invalid transport")?;
    require(
        https && args.len() == 4 || !https && args.len() == 6,
        "transport argument mismatch",
    )?;
    let action = args[1].as_str();
    require(
        matches!(action, "cancel" | "disconnect" | "complete"),
        "invalid action",
    )?;
    require(
        std::env::var_os("ROCKET_LOG_DIR").is_some(),
        "set isolated diagnostics",
    )?;
    let run = Path::new(&args[2]);
    fs::create_dir(run)?;
    let destination = run.join("clone");
    let addresses = ("github.com", if https { 443 } else { 22 })
        .to_socket_addrs()?
        .collect();
    let relay = relay::Relay::start(addresses, https)?;
    let mut config = fs::read_to_string(&args[3])?;
    if https {
        config.push_str(&format!(
            "\n[http]\n proxy = http://127.0.0.1:{}\n",
            relay.port
        ));
    } else {
        let ssh_config = run.join("relay-ssh.config");
        // OpenSSH uses the first value. HostKeyAlias keeps the real GitHub pin
        // while this process alone connects through our loopback listener.
        let ssh_text = format!(
            "Host github.com\n HostName 127.0.0.1\n Port {}\n HostKeyAlias github.com\n{}",
            relay.port,
            fs::read_to_string(&args[5])?
        );
        save(&ssh_config, &ssh_text)?;
        let command = format!(
            "{} -F {}",
            shell_quote(&args[4]),
            shell_quote(&ssh_config.to_string_lossy())
        );
        config.push_str(&format!(
            "\n[core]\n sshCommand = {}\n",
            git_quote(&command)
        ));
    }
    let config_path = run.join("relay-git.config");
    save(&config_path, &config)?;
    let executable = std::env::current_exe()?;
    let name = if action == "cancel" {
        "clone_cancel_probe"
    } else {
        "inspect"
    };
    let binary = executable.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let source = if https {
        "https://github.com/ocornut/imgui.git"
    } else {
        "ssh://git@github.com/ocornut/imgui.git"
    };
    let mut command = Command::new(binary);
    if action != "cancel" {
        command.arg("clone");
    }
    command.args(["--url", source]).arg(&destination);
    // Raw output is captured internally for fixed diagnostic flags only. Never
    // print/store the raw line; the actual CLI's normal guidance is preserved.
    if action == "disconnect" {
        command.arg("--raw-error");
    }
    command
        .env("GIT_CONFIG_GLOBAL", &config_path)
        .env("GIT_CONFIG_SYSTEM", &config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = OwnedChild(command.spawn()?);
    let (sender, receiver) = mpsc::sync_channel(128);
    let readers: Vec<Box<dyn Read + Send>> = vec![
        Box::new(child.0.stdout.take().unwrap()),
        Box::new(child.0.stderr.take().unwrap()),
    ];
    let mut handles = Vec::new();
    for reader in readers {
        let sender = sender.clone();
        handles.push(thread::spawn(move || -> io::Result<()> {
            // Cap capture independently of the child's output limits.
            let mut reader = BufReader::new(reader.take(4 * 1024 * 1024 + 1));
            let mut bytes = 0;
            loop {
                let mut line = String::new();
                let count = reader.read_line(&mut line)?;
                if count == 0 {
                    return Ok(());
                }
                bytes += count;
                if bytes > 4 * 1024 * 1024 {
                    return Err(io::Error::other("probe output limit"));
                }
                if sender.send(line).is_err() {
                    return Ok(());
                }
            }
        }));
    }
    drop(sender);
    let started = Instant::now();
    let mut trigger = None;
    let mut text = String::new();
    let mut raw = String::new();
    let mut status = None;
    let mut disconnected = false;
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => {
                if action == "disconnect" && trigger.is_none() && receiving(&line) {
                    trigger = Some(Instant::now());
                    relay.cut();
                    println!("TEST connection cut after incomplete Receiving progress");
                }
                if line.starts_with("Raw clone ")
                    || line.starts_with("stdout: ")
                    || line.starts_with("stderr: ")
                {
                    raw.push_str(&line.to_ascii_lowercase());
                } else if !line.starts_with("Raw ") {
                    print!("{line}");
                    text.push_str(&line);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
        if status.is_none() {
            status = child.0.try_wait()?;
        }
        if status.is_some() && disconnected {
            break;
        }
        require(
            started.elapsed() < Duration::from_secs(120),
            "clone acceptance exceeded 120 seconds",
        )?;
    }
    for handle in handles {
        handle.join().map_err(|_| "output reader panicked")??;
    }
    let stop_ms = trigger.map(|time| time.elapsed().as_millis()).unwrap_or(0);
    // Observe natural connection teardown before forced relay cleanup. This is
    // especially important for proving cancellation reaped the SSH helper.
    let drained = Instant::now();
    while relay.active() != 0 && drained.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    let failure = text
        .lines()
        .find_map(|line| {
            let (_, tail) = line.split_once('[')?;
            let (code, _) = tail.split_once(']')?;
            (code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')).then_some(code)
        })
        .unwrap_or("");
    let stages = fs::read_dir(run)?
        .filter_map(|v| v.ok())
        .filter(|v| {
            v.file_name()
                .to_string_lossy()
                .starts_with(".rocket-clone-")
                && v.path().is_dir()
        })
        .count();
    let exit = status.unwrap().code().unwrap_or(-1);
    let destination_absent =
        fs::symlink_metadata(&destination).is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
    let passed = relay.connections() > 0
        && relay.received() > 0
        && relay.active() == 0
        && !relay.failed()
        && match action {
            "cancel" => {
                exit == 0
                    && text.contains("PASS cancellation after receiving progress")
                    && destination_absent
                    && stages == 1
            }
            "disconnect" => {
                exit == 1
                    && trigger.is_some()
                    && stop_ms < 10_000
                    && destination_absent
                    && stages == 1
                    && text.contains("Clone verification: skipped")
                    && text.contains("Clone publication: skipped")
                    && text.contains("Next: ")
            }
            _ => {
                exit == 0
                    && destination.is_dir()
                    && stages == 0
                    && text.contains("Clone publication: succeeded")
            }
        };
    let signals: Vec<_> = [
        "connection reset by peer",
        "connection closed",
        "remote end hung up",
        "early eof",
        "curl 18",
        "curl 56",
        "curl 92",
        "bytes of body are still expected",
        "unexpected disconnect",
        "connection was aborted",
    ]
    .iter()
    .filter(|v| raw.contains(**v))
    .map(|v| format!("\"{v}\""))
    .collect();
    let summary = format!(
        "{{\"passed\":{passed},\"action\":\"{action}\",\"exit\":{exit},\"failure_code\":\"{failure}\",\"cut_after_receiving\":{},\"stop_ms\":{stop_ms},\"connections\":{},\"active_after\":{},\"encrypted_bytes_received\":{},\"retained_workspaces\":{stages},\"destination_absent\":{destination_absent},\"elapsed_ms\":{},\"relay_closures\":{:?},\"diagnostic_signals\":[{}]}}\n",
        trigger.is_some(),
        relay.connections(),
        relay.active(),
        relay.received(),
        started.elapsed().as_millis(),
        relay.closures(),
        signals.join(",")
    );
    save(&run.join("probe-result.json"), &summary)?;
    println!("PROBE {summary}");
    require(passed, "transport probe did not meet acceptance criteria")
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Transport acceptance failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_requires_incomplete_receiving_progress() {
        assert!(receiving("Clone Receiving: 1 / Some(100) objects\n"));
        for line in [
            "Clone Receiving: 100 / Some(100) objects",
            "Clone Receiving: 0 / Some(100) objects",
            "Clone Receiving: 1 / None objects",
            "Clone Counting: 1 / Some(100) objects",
        ] {
            assert!(!receiving(line));
        }
    }
}
