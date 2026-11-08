//! First-party Git SSH/AskPass helper. Never accepts host-trust or password prompts.
use super::*;
use std::{
    ffi::OsString,
    io::Write,
    process::{Command, Stdio},
};

const PROFILE_ENV: &str = "ROCKET_SSH_PROFILE";
const SECRET_ENV: &str = "ROCKET_SSH_CREDENTIAL";
const VALIDATE_ENV: &str = "ROCKET_SSH_VALIDATE";

fn profile(directory: &Path) -> Result<(SshManager, SshProfile)> {
    files::safe_path(directory)?;
    let root = directory.parent().ok_or(fail(SshFailure::InvalidProfile))?;
    let alias = directory
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(fail(SshFailure::InvalidProfile))?;
    let manager = SshManager::new(root.to_path_buf(), None)?;
    let p = manager.load(alias, &CancellationToken::new())?;
    Ok((manager, p))
}

/// Called only by rocket-ssh.exe. Errors deliberately exclude prompts/paths/secrets.
pub fn ssh_helper_main(arguments: Vec<OsString>) -> Result<i32> {
    if arguments.first().is_some_and(|a| a == "--version") && arguments.len() == 1 {
        println!("rocket-ssh {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if arguments
        .first()
        .is_some_and(|a| a == "--profile" || a == "--validate")
    {
        return launch(arguments);
    }
    if arguments.len() != 1 {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    let directory = std::env::var_os(PROFILE_ENV)
        .map(PathBuf::from)
        .ok_or(fail(SshFailure::PassphraseUnavailable))?;
    let (manager, p) = profile(&directory)?;
    let _guard = manager.guard(&p, &CancellationToken::new())?;
    let validation = std::env::var_os(VALIDATE_ENV).is_some_and(|v| v == "1");
    let prompt = arguments[0]
        .to_str()
        .ok_or(fail(SshFailure::PassphraseUnavailable))?;
    if !passphrase_prompt(prompt, &p.key, validation) {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    let path = std::env::var_os(SECRET_ENV)
        .map(PathBuf::from)
        .ok_or(fail(SshFailure::PassphraseUnavailable))?;
    if path.parent() != Some(directory.as_path())
        || (!validation && path != direct::credential_path(&p))
    {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    let secret = direct::read_secret(&p, &path)?;
    std::io::stdout()
        .lock()
        .write_all(&secret.0)
        .map_err(|_| fail(SshFailure::ToolFailed))?;
    Ok(0)
}

fn passphrase_prompt(prompt: &str, key: &Path, validation: bool) -> bool {
    let prompt = prompt.trim();
    if validation && prompt == "Enter passphrase:" {
        return true;
    }
    let key = git_path(key).to_string_lossy().replace('\\', "/");
    let prompt = prompt.replace('\\', "/");
    [
        format!("Enter passphrase for key '{key}':"),
        format!("Enter passphrase for \"{key}\":"),
    ]
    .contains(&prompt)
}

fn launch(arguments: Vec<OsString>) -> Result<i32> {
    if arguments.len() < 3 {
        return Err(fail(SshFailure::InvalidProfile));
    }
    let validation = arguments[0] == "--validate";
    let directory = PathBuf::from(&arguments[1]);
    let (manager, p) = profile(&directory)?;
    let _guard = manager.guard(&p, &CancellationToken::new())?;
    let secret = if validation {
        if arguments.len() != 3 {
            return Err(fail(SshFailure::InvalidProfile));
        }
        let path = PathBuf::from(&arguments[2]);
        if path.parent() != Some(directory.as_path())
            || !path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("pending-") && s.ends_with(".dpapi"))
        {
            return Err(fail(SshFailure::PassphraseUnavailable));
        }
        path
    } else {
        direct::credential_path(&p)
    };
    let probe = !validation && arguments[2..].iter().any(|a| a == "-G");
    let _secret_guard = if p.encrypted && !probe {
        let guard = files::locked(&secret).map_err(|_| fail(SshFailure::PassphraseUnavailable))?;
        direct::read_secret(&p, &secret)?;
        Some(guard)
    } else {
        None
    };
    let mut command = Command::new(openssh(if validation {
        "ssh-keygen.exe"
    } else {
        "ssh.exe"
    })?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    if validation {
        command
            .args(["-y", "-f"])
            .arg(git_path(&p.key))
            .stdin(Stdio::null());
    } else {
        validate_git_arguments(&arguments[2..], &p)?;
        // Binding is only a review of that moment's config. Check again in Git's
        // current directory before OpenSSH can contact a host; the command grammar
        // below also protects against changes after this check and -c overrides.
        binding::check_service_overrides(
            &GitClient::discover()?.runner,
            &std::env::current_dir().map_err(|_| fail(SshFailure::ToolFailed))?,
            &CancellationToken::new(),
        )?;
        command.arg("-F").arg(git_path(&p.config));
        for option in [
            "IdentityAgent=none",
            "IdentitiesOnly=yes",
            "AddKeysToAgent=no",
            "StrictHostKeyChecking=yes",
            "UpdateHostKeys=no",
            "VerifyHostKeyDNS=no",
            "ForwardAgent=no",
            "ClearAllForwardings=yes",
            "PasswordAuthentication=no",
            "KbdInteractiveAuthentication=no",
            "PreferredAuthentications=publickey",
            "BatchMode=no",
            "NumberOfPasswordPrompts=1",
            "ConnectTimeout=15",
            "ConnectionAttempts=1",
            "ProxyCommand=none",
            "ProxyJump=none",
            "PermitLocalCommand=no",
            "ControlMaster=no",
            "ControlPath=none",
        ] {
            command.arg("-o").arg(option);
        }
        // The selected Rocket credential is the only passphrase source in direct
        // mode. Do not silently consult or modify Apple's separate SSH Keychain.
        #[cfg(target_os = "macos")]
        command.args(["-o", "UseKeychain=no"]);
        command.args(&arguments[2..]);
    }
    command
        .env("SSH_ASKPASS", direct::helper()?)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "rocket-askpass")
        .env(PROFILE_ENV, &directory)
        .env(SECRET_ENV, &secret)
        .env(VALIDATE_ENV, if validation { "1" } else { "0" })
        .env_remove("SSH_AUTH_SOCK")
        .env_remove("SSH_AGENT_PID");
    let status = command.status().map_err(|_| fail(SshFailure::ToolFailed))?;
    Ok(status.code().unwrap_or(1))
}

fn validate_git_arguments(arguments: &[OsString], p: &SshProfile) -> Result<()> {
    let mut index = 0;
    while index < arguments.len() {
        let value = arguments[index]
            .to_str()
            .ok_or(fail(SshFailure::InvalidProfile))?;
        match value {
            "-G" | "-4" | "-6" | "-T" => index += 1,
            "-o" if arguments
                .get(index + 1)
                .is_some_and(|v| v == "SendEnv=GIT_PROTOCOL") =>
            {
                index += 2
            }
            "-p" if arguments.get(index + 1).and_then(|v| v.to_str())
                == Some(p.port.to_string().as_str()) =>
            {
                index += 2
            }
            _ => break,
        }
    }
    let destination = arguments
        .get(index)
        .and_then(|s| s.to_str())
        .ok_or(fail(SshFailure::InvalidProfile))?;
    if ![
        p.host.clone(),
        p.alias.clone(),
        format!("{}@{}", p.user, p.host),
        format!("{}@{}", p.user, p.alias),
    ]
    .contains(&destination.to_owned())
    {
        return Err(fail(SshFailure::InvalidProfile));
    }
    let rest = &arguments[index + 1..];
    let valid = match rest {
        [] => arguments[..index].iter().any(|a| a == "-G"),
        [command] => command.to_str().is_some_and(valid_remote_command),
        _ => false,
    };
    if !valid {
        return Err(fail(SshFailure::InvalidProfile));
    }
    Ok(())
}

fn valid_remote_command(command: &str) -> bool {
    if command.chars().any(char::is_control) {
        return false;
    }
    let Some((service, arguments)) = command.split_once(' ') else {
        return false;
    };
    let (path, lfs) = match service {
        "git-upload-pack" | "git-receive-pack" | "git-upload-archive" => (arguments, false),
        "git-lfs-authenticate" | "git-lfs-transfer" => {
            let Some((path, "upload" | "download")) = arguments.rsplit_once(' ') else {
                return false;
            };
            (path, true)
        }
        _ => return false,
    };
    // Git always uses sq_quote_buf: single-quoted spans joined only by escaped
    // apostrophes/exclamation marks. Nothing outside that one shell word may run.
    if let Some(mut rest) = path.strip_prefix('\'') {
        let mut first = true;
        let mut nonempty = false;
        loop {
            let Some((span, tail)) = rest.split_once('\'') else {
                return false;
            };
            if first && span.starts_with('-') {
                return false;
            }
            first = false;
            nonempty |= !span.is_empty();
            if tail.is_empty() {
                return nonempty;
            }
            let Some(next) = tail
                .strip_prefix("\\''")
                .or_else(|| tail.strip_prefix("\\!'"))
            else {
                return false;
            };
            nonempty = true;
            rest = next;
        }
    }
    // Git LFS also sends unquoted simple paths. Admit only literal characters,
    // never substitutions, shell operators, whitespace or option-like paths.
    lfs && !path.is_empty()
        && !path.starts_with('-')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-@+,:=".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile() -> SshProfile {
        SshProfile {
            alias: "rocket-test".into(),
            host: "example.invalid".into(),
            user: "git".into(),
            port: 22,
            key: PathBuf::new(),
            config: PathBuf::new(),
            fingerprint: String::new(),
            encrypted: false,
            public_key: String::new(),
            passphrase_saved: false,
            record: Vec::new(),
        }
    }

    #[test]
    fn ssh_commands_accept_git_quoted_paths_and_lfs_operations() {
        let p = test_profile();
        for command in [
            "git-upload-pack 'team/repo.git'",
            "git-receive-pack '/space and 文/repo.git'",
            "git-upload-archive '~user/repo.git'",
            r"git-upload-pack 'a'\''b'\!'c.git'",
            "git-upload-pack 'literal;$(text)`text` & | <>.git'",
            "git-lfs-authenticate team/repo.git download",
            "git-lfs-transfer 'space and 文/repo.git' upload",
        ] {
            assert!(
                validate_git_arguments(&args(&["git@example.invalid", command]), &p).is_ok(),
                "{command}"
            );
        }
        assert!(validate_git_arguments(&args(&["-G", "git@example.invalid"]), &p).is_ok());
        assert!(
            validate_git_arguments(
                &args(&[
                    "-o",
                    "SendEnv=GIT_PROTOCOL",
                    "-p",
                    "22",
                    "rocket-test",
                    "git-upload-pack 'repo'"
                ]),
                &p
            )
            .is_ok()
        );
    }

    #[test]
    fn ssh_commands_reject_extra_shell_syntax_and_invalid_paths() {
        let p = test_profile();
        for command in [
            "git-upload-pack 'repo'; echo unexpected",
            "git-receive-pack 'repo' && echo unexpected",
            "git-upload-archive 'repo' | echo unexpected",
            "git-upload-pack 'repo' > output",
            "git-upload-pack 'repo' # comment",
            "git-upload-pack $(echo unexpected)",
            "git-upload-pack `echo unexpected`",
            "git-upload-pack \"repo\"",
            "git-upload-pack repo",
            "git-upload-pack ''",
            "git-upload-pack '-option'",
            "git-upload-pack 'repo' 'extra'",
            "git-upload-pack 'repo' ",
            "git-upload-pack 'unterminated",
            "git-upload-pack 'repo\nname'",
            "git-upload-pack 'repo\rname'",
            "git-upload-pack 'repo\0name'",
            "git-upload-pack 'repo\u{7f}name'",
            "git-upload-pack 'repo'$(echo unexpected)'suffix'",
            "git-lfs-authenticate repo.git download; echo unexpected",
            "git-lfs-transfer 'repo' download extra",
            "git-lfs-transfer 'repo' invalid",
            "git-lfs-transfer 'repo'",
            "git-lfs-transfer -option upload",
            "git-lfs-transfer re$po download",
        ] {
            assert!(
                validate_git_arguments(&args(&["git@example.invalid", command]), &p).is_err(),
                "accepted {command:?}"
            );
        }
        assert!(validate_git_arguments(&args(&["git@example.invalid"]), &p).is_err());
        assert!(
            validate_git_arguments(
                &args(&["git@example.invalid", "git-upload-pack 'repo'", "extra"]),
                &p
            )
            .is_err()
        );
    }

    #[test]
    fn passphrase_prompts_never_accept_hosts_passwords_or_other_keys() {
        let key = Path::new("C:/keys/my key");
        assert!(passphrase_prompt(
            "Enter passphrase for key 'C:/keys/my key': ",
            key,
            false
        ));
        assert!(passphrase_prompt("Enter passphrase:", key, true));
        for text in [
            "Enter passphrase:",
            "Enter passphrase for key 'C:/keys/other':",
            "The authenticity of host cannot be established. Continue?",
            "git@host's password:",
        ] {
            assert!(!passphrase_prompt(text, key, false));
        }
    }
}
