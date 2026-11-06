//! Windows OpenSSH profiles. Private key material is never logged or returned.
mod binding;
mod discovery;
mod direct;
mod launcher;
mod files;
pub use direct::SshAuthentication;
pub use launcher::ssh_helper_main;
use crate::{
    local_files,
    runner::{Runner, args, git_path},
    *,
};
pub use binding::SshBindingReview;
pub use discovery::{SshDiscoveredKey, SshDiscovery};
use files::{create_file, create_private_dir, pin, read};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshFailure {
    Unavailable,
    InvalidProfile,
    Exists,
    UnsafePath,
    Changed,
    KeyInvalid,
    Unencrypted,
    AgentUnavailable,
    UnlockFailed,
    ToolFailed,
    RecoveryRequired,
    NoRecovery,
    Overrides,
    PassphraseUnavailable,
    HelperUnavailable,
}
impl SshFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "ssh_tools_unavailable",
            Self::InvalidProfile => "ssh_profile_invalid",
            Self::Exists => "ssh_profile_exists",
            Self::UnsafePath => "ssh_unsafe_path",
            Self::Changed => "ssh_external_change",
            Self::KeyInvalid => "ssh_key_invalid",
            Self::Unencrypted => "ssh_encryption_required",
            Self::AgentUnavailable => "ssh_agent_unavailable",
            Self::UnlockFailed => "ssh_unlock_failed",
            Self::ToolFailed => "ssh_tool_failed",
            Self::RecoveryRequired => "ssh_recovery_required",
            Self::NoRecovery => "ssh_no_matching_recovery",
            Self::Overrides => "ssh_config_overrides",
            Self::PassphraseUnavailable => "ssh_passphrase_unavailable",
            Self::HelperUnavailable => "ssh_helper_unavailable",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Unavailable => {
                "Windows OpenSSH Client is unavailable. Enable the Windows optional feature, then refresh."
            }
            Self::PassphraseUnavailable => "No usable saved passphrase exists for this exact key and profile. Open SSH keys, select the key, and verify/save its passphrase, or use Windows agent mode.",
            Self::HelperUnavailable => "Rocket's SSH helper is missing or invalid. Use a complete Rocket bundle; existing SSH settings and keys were preserved.",
            Self::InvalidProfile => {
                "Use a unique rocket- alias, a DNS hostname, a simple SSH username and a port from 1 to 65535."
            }
            Self::Exists => {
                "That profile folder already exists. Choose a new alias; Rocket never overwrites existing keys or profiles."
            }
            Self::UnsafePath => {
                "Choose a local regular file or folder without links, junctions, or SSH expansion characters (quotes, %, $, or backtick). Existing data was preserved."
            }
            Self::Changed => {
                "The key, profile or repository settings changed externally. Refresh and review again. Nothing was overwritten."
            }
            Self::KeyInvalid => {
                "The public key and OpenSSH private key must be a matching pair. Choose the private file with its adjacent .pub file. PEM and security-token formats are not supported by this manager."
            }
            Self::Unencrypted => {
                "New keys must have a passphrase of at least 10 characters. No unencrypted key was published."
            }
            Self::AgentUnavailable => {
                "Windows ssh-agent is unavailable. Start the ssh-agent service in Windows, then refresh. Rocket does not change service settings."
            }
            Self::UnlockFailed => {
                "OpenSSH could not unlock this key. Check the passphrase and file permissions, then retry. Existing keys were preserved."
            }
            Self::ToolFailed => {
                "OpenSSH did not finish successfully. Check the key and Windows OpenSSH installation, then refresh. Raw tool output is not logged."
            }
            Self::RecoveryRequired => {
                "The operation finished only partially. Inspect the retained profile folder or repository SSH recovery backup before retrying; Rocket will not overwrite it."
            }
            Self::Overrides => {
                "Git has custom remote-service commands, include/worktree SSH settings or an inherited GIT_SSH override. Review those settings with Git before using this profile; Rocket preserved all settings."
            }
            Self::NoRecovery => {
                "No backup matches the current repository config. External edits were preserved. Use the saved recovery files for a manual review."
            }
        }
    }
}
fn fail(f: SshFailure) -> Error {
    Error::Ssh(f)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshProfile {
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    pub key: PathBuf,
    pub config: PathBuf,
    pub fingerprint: String,
    pub encrypted: bool,
    pub public_key: String,
    pub passphrase_saved: bool,
    record: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct SshInventory {
    pub profiles: Vec<SshProfile>,
    pub agent_running: bool,
    pub loaded: Vec<String>,
    pub invalid: usize,
}
#[derive(Clone)]
pub struct SshManager {
    root: PathBuf,
    runner: Runner,
}
pub struct SshPrepared {
    manager: SshManager,
    pub directory: PathBuf,
    pub key: PathBuf,
    alias: String,
    host: String,
    user: String,
    port: u16,
    generated: bool,
    rsa: bool,
    published: bool,
    _parents: Vec<fs::File>,
}
pub struct SshKeyGuard {
    _files: Vec<fs::File>,
    _parents: Vec<fs::File>,
}
impl SshManager {
    pub fn new(root: PathBuf, log: Option<DiagnosticLog>) -> Result<Self> {
        files::safe_path(&root)?;
        let executable = openssh("ssh-keygen.exe")?;
        Ok(Self {
            root,
            runner: Runner {
                executable,
                log,
                transport_scope: None,
                secret_protection: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                guard_ai_api_keys: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                push_check: Default::default(),
                git_http_allowed: false, // This runner launches only OpenSSH tools.
                managed_git: None,
                limits: CommandLimits {
                    timeout: Duration::from_secs(10),
                    stdout_bytes: 64 * 1024,
                    stderr_bytes: 16 * 1024,
                },
            },
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn inventory(&self, cancel: &CancellationToken) -> Result<SshInventory> {
        self.runner.step("ssh-list", "read_ssh_profiles", || {
            cancel.check()?;
            let mut profiles = Vec::new();
            let mut invalid = 0;
            if self.root.exists() {
                let _pin = pin(&self.root)?;
                for entry in fs::read_dir(&self.root)
                    .map_err(|e| Error::io("read SSH profiles", e))?
                    .take(129)
                {
                    let entry = entry.map_err(|e| Error::io("read SSH profile", e))?;
                    cancel.check()?;
                    if profiles.len() + invalid >= 128 {
                        return Err(fail(SshFailure::InvalidProfile));
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    match self.load(&name, cancel) {
                        Ok(p) => profiles.push(p),
                        Err(_) => invalid += 1,
                    }
                }
            }
            profiles.sort_by(|a, b| a.alias.cmp(&b.alias));
            let (agent_running, loaded) = self.agent(cancel)?;
            Ok(SshInventory {
                profiles,
                invalid,
                agent_running,
                loaded,
            })
        })
    }
    fn tool(
        &self,
        name: &str,
        arguments: Vec<std::ffi::OsString>,
        cancel: &CancellationToken,
    ) -> Result<crate::runner::Output> {
        let exe = openssh(name)?;
        self.runner.tool(
            &exe,
            if name == "ssh-add.exe" {
                "ssh-add"
            } else if name == "ssh.exe" {
                "ssh-config"
            } else {
                "ssh-keygen"
            },
            exe.parent().ok_or(fail(SshFailure::Unavailable))?,
            &arguments,
            vec![],
            cancel,
        )
    }
    pub fn agent(&self, cancel: &CancellationToken) -> Result<(bool, Vec<String>)> {
        let out = self.tool("ssh-add.exe", args(&["-l", "-E", "sha256"]), cancel)?;
        let running = out.status.code() == Some(0) || out.status.code() == Some(1);
        let loaded = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1))
            .filter(|s| s.starts_with("SHA256:"))
            .map(str::to_owned)
            .collect();
        Ok((running, loaded))
    }
    pub fn load(&self, alias: &str, cancel: &CancellationToken) -> Result<SshProfile> {
        if !valid_alias(alias) {
            return Err(fail(SshFailure::InvalidProfile));
        }
        let directory = self.root.join(alias);
        let _pin = pin(&directory)?;
        let record = read(&directory.join("profile"), 8192)?;
        let text = std::str::from_utf8(&record).map_err(|_| fail(SshFailure::InvalidProfile))?;
        let fields: Vec<_> = text.lines().collect();
        if fields.len() != 7 || fields[0] != "ROCKET-SSH-1" || fields[1] != alias {
            return Err(fail(SshFailure::InvalidProfile));
        }
        let port = fields[4]
            .parse()
            .map_err(|_| fail(SshFailure::InvalidProfile))?;
        validate(alias, fields[2], fields[3], port)?;
        let key = PathBuf::from(fields[5]);
        let (public_key, encrypted) = files::key_pair(&key)?;
        let fingerprint = self.fingerprint(&key, cancel)?;
        if fingerprint != fields[6] {
            return Err(fail(SshFailure::Changed));
        }
        let config = directory.join("config");
        if read(&config, 16384)?
            != configuration(alias, fields[2], fields[3], port, &key)?.as_bytes()
        {
            return Err(fail(SshFailure::Changed));
        }
        Ok(SshProfile {
            alias: alias.into(),
            host: fields[2].into(),
            user: fields[3].into(),
            port,
            key,
            config,
            fingerprint,
            encrypted,
            public_key,
            passphrase_saved: directory.join("passphrase.dpapi").exists(),
            record,
        })
    }
    fn fingerprint(&self, key: &Path, cancel: &CancellationToken) -> Result<String> {
        let mut a = args(&["-l", "-E", "sha256", "-f"]);
        a.push(git_path(&files::public_path(key)));
        let out = self.tool("ssh-keygen.exe", a, cancel)?;
        if !out.status.success() {
            return Err(fail(SshFailure::KeyInvalid));
        }
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .nth(1)
            .filter(|s| s.starts_with("SHA256:") && s.len() < 100)
            .map(str::to_owned)
            .ok_or(fail(SshFailure::KeyInvalid))
    }
    pub fn prepare(
        &self,
        alias: &str,
        host: &str,
        user: &str,
        port: u16,
        existing: Option<&Path>,
        cancel: &CancellationToken,
    ) -> Result<SshPrepared> {
        self.prepare_with_algorithm(alias, host, user, port, existing, false, cancel)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_with_algorithm(
        &self,
        alias: &str,
        host: &str,
        user: &str,
        port: u16,
        existing: Option<&Path>,
        rsa: bool,
        cancel: &CancellationToken,
    ) -> Result<SshPrepared> {
        self.runner.step("ssh-create", "prepare_ssh_profile", || {
            cancel.check()?;
            validate(alias, host, user, port)?;
            if let Some(key) = existing {
                files::key_pair(key)?;
            }
            // Create only missing manager folders. Existing directories are pinned, not re-permissioned.
            files::ensure_private_parent(&self.root)?;
            let mut parents = pin(&self.root)?;
            let directory = self.root.join(alias);
            if fs::symlink_metadata(&directory).is_ok() {
                return Err(fail(SshFailure::Exists));
            }
            create_private_dir(&directory)?;
            parents.extend(pin(&directory)?);
            let key = existing
                .map(Path::to_path_buf)
                .unwrap_or_else(|| directory.join(if rsa { "id_rsa" } else { "id_ed25519" }));
            Ok(SshPrepared {
                manager: self.clone(),
                directory,
                key,
                alias: alias.into(),
                host: host.into(),
                user: user.into(),
                port,
                generated: existing.is_none(),
                rsa,
                published: false,
                _parents: parents,
            })
        })
    }
    pub fn guard(&self, profile: &SshProfile, cancel: &CancellationToken) -> Result<SshKeyGuard> {
        let mut parents = pin(profile.key.parent().ok_or(fail(SshFailure::UnsafePath))?)?;
        parents.extend(pin(profile
            .config
            .parent()
            .ok_or(fail(SshFailure::UnsafePath))?)?);
        let files = vec![
            files::locked(&profile.key)?,
            files::locked(&files::public_path(&profile.key))?,
            files::locked(&profile.config)?,
            files::locked(&profile.config.with_file_name("profile"))?,
        ];
        if self.load(&profile.alias, cancel)? != *profile {
            return Err(fail(SshFailure::Changed));
        }
        Ok(SshKeyGuard {
            _files: files,
            _parents: parents,
        })
    }
    pub fn unload(&self, profile: &SshProfile, cancel: &CancellationToken) -> Result<()> {
        self.runner
            .step("ssh-unload", "remove_selected_identity", || {
                let _guard = self.guard(profile, cancel)?;
                self.remove_identity(&profile.key, &profile.fingerprint, cancel)
            })
    }
    /// Remove one key's identity from ssh-agent; the caller holds the key's files.
    fn remove_identity(
        &self,
        key: &Path,
        fingerprint: &str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if !self.agent(cancel)?.0 {
            return Err(fail(SshFailure::AgentUnavailable));
        }
        let mut a = args(&["-k", "-d"]);
        a.push(git_path(&files::public_path(key)));
        let attempt = self.tool("ssh-add.exe", a, cancel);
        // Reconcile the one selected fingerprint even after a helper error.
        let state = self.agent(&CancellationToken::new())?;
        if !state.0 {
            return Err(fail(SshFailure::AgentUnavailable));
        }
        if !state.1.iter().any(|f| f == fingerprint) {
            return Ok(());
        }
        attempt?;
        cancel.check()?;
        Err(fail(SshFailure::ToolFailed))
    }
    pub fn check_config(&self, p: &SshProfile, cancel: &CancellationToken) -> Result<String> {
        let _guard = self.guard(p, cancel)?;
        let mut a = args(&["-G", "-F"]);
        a.push(git_path(&p.config));
        a.push(p.alias.clone().into());
        let out = self.tool("ssh.exe", a, cancel)?;
        if !out.status.success() {
            return Err(fail(SshFailure::InvalidProfile));
        }
        let s = String::from_utf8_lossy(&out.stdout);
        for line in [
            format!("hostname {}", p.host),
            format!("user {}", p.user),
            format!("port {}", p.port),
            "identitiesonly yes".into(),
        ] {
            if !s.lines().any(|l| l == line) {
                return Err(fail(SshFailure::InvalidProfile));
            }
        }
        Ok(format!(
            "Windows OpenSSH: {}@{}:{} · IdentitiesOnly yes",
            p.user, p.host, p.port
        ))
    }
}
impl SshPrepared {
    pub fn arguments(&self) -> Vec<String> {
        let mut a = vec![
            "-t".into(),
            if self.rsa { "rsa" } else { "ed25519" }.into(),
            "-a".into(),
            "64".into(),
            "-C".into(),
            "Rocket".into(),
            "-f".into(),
            git_path(&self.key).to_string_lossy().into_owned(),
        ];
        if self.rsa {
            a.extend(["-b".into(), "4096".into()]);
        }
        a
    }
    pub fn finish(mut self, cancel: &CancellationToken) -> Result<SshProfile> {
        self.manager
            .runner
            .step("ssh-create", "publish_ssh_profile", || {
                cancel.check()?;
                let (_, encrypted) = files::key_pair(&self.key)?;
                if self.generated && !encrypted {
                    return Err(fail(SshFailure::Unencrypted));
                }
                let fingerprint = self.manager.fingerprint(&self.key, cancel)?;
                let config =
                    configuration(&self.alias, &self.host, &self.user, self.port, &self.key)?;
                create_file(&self.directory.join("config"), config.as_bytes())?;
                let record = format!(
                    "ROCKET-SSH-1\n{}\n{}\n{}\n{}\n{}\n{}\n",
                    self.alias,
                    self.host,
                    self.user,
                    self.port,
                    git_path(&self.key).to_string_lossy(),
                    fingerprint
                );
                cancel.check()?;
                create_file(&self.directory.join("profile"), record.as_bytes())?;
                self.published = true;
                self.manager.load(&self.alias, &CancellationToken::new())
            })
    }
}
impl Drop for SshPrepared {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // Remove only an empty, exclusively created folder. Partial key files are
        // retained privately for recovery; never guess whether an external edit is ours.
        self._parents.clear();
        let _ = fs::remove_dir(&self.directory);
    }
}
fn valid_alias(s: &str) -> bool {
    s.starts_with("rocket-")
        && s.len() <= 64
        && s.len() > 7
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn validate(alias: &str, host: &str, user: &str, port: u16) -> Result<()> {
    if !valid_alias(alias)
        || host.is_empty()
        || host.len() > 253
        || port == 0
        || host.split('.').any(|p| {
            p.is_empty()
                || p.starts_with('-')
                || p.ends_with('-')
                || !p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
        || user.is_empty()
        || user.len() > 64
        || !user
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(fail(SshFailure::InvalidProfile));
    }
    Ok(())
}
fn configuration(alias: &str, host: &str, user: &str, port: u16, key: &Path) -> Result<String> {
    files::safe_path(key)?;
    let key = git_path(key).to_string_lossy().replace('\\', "/");
    Ok(format!(
        "# Rocket SSH profile. No user/system SSH config is inherited.\nHost {alias} {host}\n    HostName {host}\n    User {user}\n    Port {port}\n    IdentityFile \"{key}\"\n    IdentitiesOnly yes\nHost *\n    IdentitiesOnly yes\n    IdentityFile none\n"
    ))
}
pub fn openssh(tool: &str) -> Result<PathBuf> {
    if !matches!(tool, "ssh-keygen.exe" | "ssh-add.exe" | "ssh.exe") {
        return Err(fail(SshFailure::Unavailable));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetSystemDirectoryW(b: *mut u16, n: u32) -> u32;
        }
        let mut b = vec![0u16; 32768];
        let n = unsafe { GetSystemDirectoryW(b.as_mut_ptr(), b.len() as u32) } as usize;
        if n == 0 || n >= b.len() {
            return Err(fail(SshFailure::Unavailable));
        }
        let p = PathBuf::from(std::ffi::OsString::from_wide(&b[..n]))
            .join("OpenSSH")
            .join(tool);
        if !p.is_file() {
            return Err(fail(SshFailure::Unavailable));
        }
        Ok(p)
    }
    #[cfg(not(windows))]
    {
        Err(fail(SshFailure::Unavailable))
    }
}
