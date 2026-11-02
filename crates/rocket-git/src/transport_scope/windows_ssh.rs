//! An operator-selected, immutable SSH profile; repository config never selects commands.
use crate::{Error, Result, local_files, runner::git_path};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

// GitHub's published Ed25519 host key, verified against its official fingerprint
// page. Rotation must be reviewed; neither DNS nor a first connection can replace it.
const HOST_KEY: &str =
    "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n";
const SOURCES: [&str; 2] = [
    "git@github.com:JasXTD/rocket-auth-test.git",
    "ssh://git@github.com/JasXTD/rocket-auth-test.git",
];
fn approved_source(url: &str) -> bool {
    SOURCES.contains(&url) || url.strip_prefix("git@github.com:").and_then(|p|p.strip_suffix(".git"))
        .is_some_and(|repository| crate::ForgeConfig::new(crate::ForgeProvider::GitHub,"https://github.com")
            .and_then(|c|c.with_github_publish_check(repository)).is_ok())
}

#[derive(Debug)]
pub(super) struct WindowsSsh {
    command: String,
    _files: Vec<File>,
    _directories: Vec<File>,
}

fn refused() -> Error {
    Error::InvalidInput("The scoped Windows SSH agent profile requires a regular in-scope Ed25519 public key and the approved GitHub test repository".into())
}

fn pin(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| refused())?;
    let metadata = file.metadata().map_err(|_| refused())?;
    if !metadata.is_file() || local_files::redirected(&metadata) {
        return Err(refused());
    }
    Ok(file)
}

fn public_key(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| refused())?;
    let mut lines = text.lines();
    let fields = lines
        .next()
        .ok_or_else(refused)?
        .split_whitespace()
        .collect::<Vec<_>>();
    if lines.any(|line| !line.is_empty())
        || fields.len() < 2
        || fields[0] != "ssh-ed25519"
        || fields[1].len() != 68
        || !fields[1].starts_with("AAAAC3NzaC1lZDI1NTE5AAAAI")
        || !fields[1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    {
        return Err(refused());
    }
    // Comments (which can contain a personal identity) are neither needed nor copied.
    Ok(format!("{} {}\n", fields[0], fields[1]))
}

fn ssh_path(path: &Path) -> Result<String> {
    let text = git_path(path)
        .into_string()
        .map_err(|_| refused())?
        .replace('\\', "/");
    // SSH expands percent/dollar tokens even in quoted config values.
    if text.chars().any(|c| c.is_control() || "\"%$`".contains(c)) {
        return Err(refused());
    }
    Ok(format!("\"{text}\""))
}

fn shell_path(path: &Path) -> Result<String> {
    let text = git_path(path)
        .into_string()
        .map_err(|_| refused())?
        .replace('\\', "/");
    if text.chars().any(char::is_control) {
        return Err(refused());
    }
    Ok(format!("'{}'", text.replace('\'', "'\\''")))
}

fn create_pinned(path: &Path, bytes: &[u8]) -> Result<File> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| refused())?;
    file.write_all(bytes).map_err(|_| refused())?;
    drop(file);
    let mut file = pin(path)?;
    let mut observed = Vec::new();
    Read::by_ref(&mut file)
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut observed)
        .map_err(|_| refused())?;
    if observed != bytes {
        return Err(refused());
    }
    Ok(file)
}

impl WindowsSsh {
    pub(super) fn new(root: &Path, urls: &[String], public: &Path) -> Result<Self> {
        if !cfg!(windows)
            || urls
                .iter()
                .any(|url| !url.starts_with("https://") && !approved_source(url))
            || !public.is_absolute()
            || public.extension().is_none_or(|ext| ext != "pub")
        {
            return Err(refused());
        }
        let public = PathBuf::from(git_path(public));
        let lexical_root = PathBuf::from(git_path(root));
        let source_parent = public.parent().ok_or_else(refused)?;
        let mut directories = local_files::pin_directories(&lexical_root, source_parent)?;
        let mut input = pin(&public)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut input)
            .take(8193)
            .read_to_end(&mut bytes)
            .map_err(|_| refused())?;
        if bytes.len() > 8192 {
            return Err(refused());
        }
        let identity = public_key(&bytes)?;
        let executable = crate::ssh_keys::openssh("ssh.exe")?;
        let executable_guard = pin(&executable)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| refused())?
            .as_nanos();
        let directory = root.join(format!(
            ".rocket-trusted-ssh-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).map_err(|_| refused())?;
        directories.extend(local_files::pin_directories(root, &directory)?);
        let known = directory.join("known_hosts");
        let public = directory.join("identity.pub");
        let config = directory.join("ssh.config");
        let contents = format!(
            "Host *\n HostName github.com\n User git\n Port 22\n BatchMode yes\n ConnectTimeout 10\n ConnectionAttempts 1\n StrictHostKeyChecking yes\n HostKeyAlgorithms ssh-ed25519\n UserKnownHostsFile {}\n GlobalKnownHostsFile none\n UpdateHostKeys no\n VerifyHostKeyDNS no\n PreferredAuthentications publickey\n PasswordAuthentication no\n KbdInteractiveAuthentication no\n IdentitiesOnly yes\n IdentityFile {}\n IdentityAgent SSH_AUTH_SOCK\n AddKeysToAgent no\n ForwardAgent no\n ClearAllForwardings yes\n ControlMaster no\n ControlPath none\n ProxyCommand none\n ProxyJump none\n PermitLocalCommand no\n CanonicalizeHostname no\n RequestTTY no\n",
            ssh_path(&known)?,
            ssh_path(&public)?
        );
        let files = vec![
            executable_guard,
            create_pinned(&known, HOST_KEY.as_bytes())?,
            create_pinned(&public, identity.as_bytes())?,
            create_pinned(&config, contents.as_bytes())?,
        ];
        Ok(Self {
            command: format!("{} -F {}", shell_path(&executable)?, shell_path(&config)?),
            _files: files,
            _directories: directories,
        })
    }

    pub(super) fn configure(&self, command: &mut Command) {
        // Git/LFS child Git inherit only this application-built command. This is
        // applied after generic environment cleanup, never accepted from config.
        command
            .env("GIT_SSH_COMMAND", &self.command)
            .env("GIT_SSH_VARIANT", "ssh")
            .env("SSH_AUTH_SOCK", r"\\.\pipe\openssh-ssh-agent")
            .env_remove("SSH_AGENT_PID");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_publish_ssh_scope_refuses_other_hosts_and_repository_names() {
        assert!(approved_source("git@github.com:Tester/rocket-publish-test-0123456789abcdef0123456789abcdef.git"));
        for bad in ["git@github.com:Tester/ordinary.git", "git@github.com.invalid:Tester/rocket-publish-test-0123456789abcdef0123456789abcdef.git", "git@github.com:Tester/rocket-publish-test-0123456789abcdef0123456789abcdef.git/extra", "ssh://git@github.com/Tester/rocket-publish-test-0123456789abcdef0123456789abcdef.git"] {
            assert!(!approved_source(bad));
        }
    }
    #[test]
    fn profile_input_is_public_data_without_ssh_directives_or_identity_comments() {
        let key = HOST_KEY.split_once(' ').unwrap().1;
        assert_eq!(
            public_key(format!("{} fixture-comment\n", key.trim()).as_bytes()).unwrap(),
            key
        );
        for bad in [
            format!("{key}ProxyCommand bad\n"),
            "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
            "ssh-ed25519 invalid".into(),
        ] {
            assert!(public_key(bad.as_bytes()).is_err());
        }
        for bad in ["C:/fixture/%h", "C:/fixture/$HOME", "C:/fixture/\"bad"] {
            assert!(ssh_path(Path::new(bad)).is_err());
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_profile_pins_trust_and_selects_only_the_system_agent() {
        use std::os::windows::process::CommandExt;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../work/scoped-ssh-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let root = PathBuf::from(git_path(&root.canonicalize().unwrap()));
        let public = root.join("fixture.pub");
        fs::write(&public, HOST_KEY.split_once(' ').unwrap().1).unwrap();
        let urls = SOURCES.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let profile = WindowsSsh::new(&root, &urls, &public).unwrap();
        assert!(WindowsSsh::new(&root, &["git@other.invalid:test.git".into()], &public).is_err());
        assert!(WindowsSsh::new(&root, &urls, &root.join("../outside.pub")).is_err());
        let dir = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .unwrap();
        let config = dir.join("ssh.config");
        for file in [
            config.clone(),
            dir.join("known_hosts"),
            dir.join("identity.pub"),
        ] {
            assert!(
                fs::write(&file, "changed").is_err(),
                "profile must stay immutable"
            );
            assert!(fs::rename(&file, file.with_extension("moved")).is_err());
        }
        let mut command = Command::new(crate::ssh_keys::openssh("ssh.exe").unwrap());
        profile.configure(&mut command);
        command
            .args(["-G", "-F"])
            .arg(&config)
            .arg("github.com")
            .creation_flags(0x0800_0000);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let effective = String::from_utf8(output.stdout)
            .unwrap()
            .to_ascii_lowercase();
        for expected in [
            "hostname github.com",
            "user git",
            "port 22",
            "identitiesonly yes",
            "batchmode yes",
            "stricthostkeychecking true",
            "hostkeyalgorithms ssh-ed25519",
            "forwardagent no",
            "passwordauthentication no",
            "globalknownhostsfile none",
        ] {
            assert!(
                effective.lines().any(|line| line == expected),
                "missing {expected}: {effective}"
            );
        }
        assert_eq!(
            effective
                .lines()
                .filter(|line| line.starts_with("identityfile "))
                .count(),
            1
        );
        assert!(effective.contains("identity.pub"));
        assert!(effective.contains("identityagent ssh_auth_sock"));
        assert_eq!(
            fs::read_to_string(dir.join("known_hosts")).unwrap(),
            HOST_KEY
        );
        drop(profile);
        assert!(
            OpenOptions::new().write(true).open(&config).is_ok(),
            "pins must release with the scope"
        );
    }
}
