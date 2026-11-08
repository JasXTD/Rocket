//! Explicit, profile-bound direct-key credentials. Only ciphertext is persisted.
use super::*;
use files::SecretBytes;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SshAuthentication {
    #[default]
    Agent,
    Direct,
}

pub(super) fn helper() -> Result<PathBuf> {
    let executable = std::env::current_exe().map_err(|_| fail(SshFailure::HelperUnavailable))?;
    let mut directory = executable
        .parent()
        .ok_or(fail(SshFailure::HelperUnavailable))?;
    // Cargo integration tests run in deps; shipped executables live together.
    if directory.file_name().is_some_and(|n| n == "deps") {
        directory = directory
            .parent()
            .ok_or(fail(SshFailure::HelperUnavailable))?;
    }
    let path = directory.join(format!("rocket-ssh{}", std::env::consts::EXE_SUFFIX));
    files::locked(&path).map_err(|_| fail(SshFailure::HelperUnavailable))?;
    Ok(path)
}

fn secret_path(profile: &SshProfile) -> PathBuf {
    profile.config.with_file_name("passphrase.dpapi")
}

struct Pending(PathBuf);
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

impl SshManager {
    pub fn check_authentication(
        &self,
        profile: &SshProfile,
        mode: SshAuthentication,
        cancel: &CancellationToken,
    ) -> Result<String> {
        if mode == SshAuthentication::Agent {
            return self.check_config(profile, cancel);
        }
        let _guard = self.guard(profile, cancel)?;
        let args = vec![
            "--profile".into(),
            git_path(profile.config.parent().unwrap()),
            "-G".into(),
            format!("{}@{}", profile.user, profile.host).into(),
        ];
        let out = self.runner.tool(
            &helper()?,
            "ssh-config",
            profile.config.parent().unwrap(),
            &args,
            vec![],
            cancel,
        )?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !out.status.success()
            || !text.lines().any(|s| s == "identityagent none")
            || !text
                .lines()
                .any(|s| s == "stricthostkeychecking true" || s == "stricthostkeychecking yes")
        {
            return Err(fail(SshFailure::InvalidProfile));
        }
        Ok(format!(
            "Direct key: {}@{}:{} · agent disabled · host verification required",
            profile.user, profile.host, profile.port
        ))
    }
    /// Validate through OpenSSH before publication. The passphrase is never an
    /// argument, environment value, log field or plaintext temporary file.
    pub fn remember_passphrase(
        &self,
        profile: &SshProfile,
        passphrase: String,
        cancel: &CancellationToken,
    ) -> Result<SshProfile> {
        let secret = SecretBytes(passphrase.into_bytes());
        self.runner
            .step("ssh-remember", "verify_and_protect_ssh_passphrase", || {
                cancel.check()?;
                let _guard = self.guard(profile, cancel)?;
                let _gate = CredentialGate::new(profile)?;
                let target = secret_path(profile);
                if target.exists() {
                    return Err(fail(SshFailure::Changed));
                }
                if secret.0.is_empty()
                    || secret.0.len() > 1024
                    || std::str::from_utf8(&secret.0)
                        .map_or(true, |s| s.chars().any(char::is_control))
                {
                    return Err(fail(SshFailure::UnlockFailed));
                }
                let mut plaintext = SecretBytes(profile.record.clone());
                plaintext.0.push(0);
                plaintext.0.extend_from_slice(&secret.0);
                let encrypted = crate::forge::protect_hosting_secret(&plaintext.0, true)
                    .map_err(|_| fail(SshFailure::PassphraseUnavailable))?;
                let pending = profile
                    .config
                    .with_file_name(format!("pending-{}.dpapi", binding::unique()));
                create_file(&pending, &encrypted)?;
                let mut owned = Pending(pending.clone());
                let arguments = vec![
                    "--validate".into(),
                    git_path(profile.config.parent().unwrap()),
                    git_path(&pending),
                ];
                let out = self.runner.tool(
                    &helper()?,
                    "ssh-keygen",
                    profile.config.parent().unwrap(),
                    &arguments,
                    vec![],
                    cancel,
                )?;
                if !out.status.success() {
                    return Err(fail(SshFailure::UnlockFailed));
                }
                // Success alone is insufficient: verify the derived public key too.
                let expected: Vec<_> = profile.public_key.split_whitespace().take(2).collect();
                let actual =
                    std::str::from_utf8(&out.stdout).map_err(|_| fail(SshFailure::KeyInvalid))?;
                if actual.split_whitespace().take(2).collect::<Vec<_>>() != expected {
                    return Err(fail(SshFailure::KeyInvalid));
                }
                cancel.check()?;
                // Exclusive on every supported OS: a concurrent writer's saved
                // credential must survive. The profile/key remain pinned here.
                publish(&pending, &target)?;
                owned.0 = PathBuf::new();
                self.load(&profile.alias, &CancellationToken::new())
            })
    }

    pub fn forget_passphrase(
        &self,
        profile: &SshProfile,
        cancel: &CancellationToken,
    ) -> Result<SshProfile> {
        self.runner
            .step("ssh-forget", "forget_selected_ssh_passphrase", || {
                cancel.check()?;
                let _guard = self.guard(profile, cancel)?;
                let _gate = CredentialGate::new(profile)?;
                let path = secret_path(profile);
                // Refuse links and nonfiles even though only this one pathname is removed.
                let _ = read(&path, 32768)?;
                cancel.check()?;
                fs::remove_file(&path).map_err(|e| Error::io("forget saved SSH passphrase", e))?;
                self.load(&profile.alias, &CancellationToken::new())
            })
    }
}

fn publish(pending: &Path, target: &Path) -> Result<()> {
    local_files::move_path(pending, target, false).map_err(|_| fail(SshFailure::Changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_publication_preserves_concurrent_files() {
        let directory = std::env::temp_dir().join(format!(
            "rocket-credential-publication-{}",
            binding::unique()
        ));
        fs::create_dir(&directory).unwrap();
        let pending = directory.join("pending");
        let target = directory.join("saved");
        fs::write(&pending, b"new ciphertext fixture").unwrap();
        fs::write(&target, b"concurrent ciphertext fixture").unwrap();
        assert!(matches!(
            publish(&pending, &target),
            Err(Error::Ssh(SshFailure::Changed))
        ));
        assert_eq!(fs::read(&target).unwrap(), b"concurrent ciphertext fixture");
        assert_eq!(fs::read(&pending).unwrap(), b"new ciphertext fixture");
        fs::remove_file(&target).unwrap();
        #[cfg(unix)]
        {
            let external = directory.join("external");
            fs::write(&external, b"external fixture").unwrap();
            std::os::unix::fs::symlink(&external, &target).unwrap();
            assert!(matches!(
                publish(&pending, &target),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert_eq!(fs::read(&external).unwrap(), b"external fixture");
            fs::remove_file(&target).unwrap();
            fs::remove_file(&external).unwrap();
        }
        publish(&pending, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new ciphertext fixture");
        assert!(!pending.exists());
        fs::remove_file(target).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}

/// Serializes credential changes without locking or editing the actual key.
struct CredentialGate {
    _file: fs::File,
}
impl CredentialGate {
    fn new(p: &SshProfile) -> Result<Self> {
        let path = p.config.with_file_name("passphrase.lock");
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            local_files::no_follow(&mut options);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Do not follow reparse points or allow the pathname to be replaced
            // while a writer holds it. The surrounding profile guard pins parents.
            options.share_mode(3).custom_flags(0x0020_0000);
        }
        let file = options.open(&path).map_err(|_| fail(SshFailure::Changed))?;
        let meta = file.metadata().map_err(|_| fail(SshFailure::Changed))?;
        if !meta.is_file() || local_files::redirected(&meta) || meta.len() != 0 {
            return Err(fail(SshFailure::UnsafePath));
        }
        #[cfg(unix)]
        if !local_files::opened_without_link(&file, &path)
            .map_err(|_| fail(SshFailure::UnsafePath))?
        {
            return Err(fail(SshFailure::UnsafePath));
        }
        // Keep the empty lock file. Closing this non-inheritable handle also
        // releases the OS lock after a crash; no stale-file deletion is needed.
        file.try_lock().map_err(|_| fail(SshFailure::Changed))?;
        Ok(Self { _file: file })
    }
}

pub(super) fn read_secret(p: &SshProfile, path: &Path) -> Result<SecretBytes> {
    if path.parent() != p.config.parent() {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    let bytes = read(path, 32768).map_err(|_| fail(SshFailure::PassphraseUnavailable))?;
    let decoded = SecretBytes(
        crate::forge::protect_hosting_secret(&bytes, false)
            .map_err(|_| fail(SshFailure::PassphraseUnavailable))?,
    );
    let prefix = p.record.len();
    if decoded.0.len() <= prefix + 1
        || decoded.0.get(..prefix) != Some(p.record.as_slice())
        || decoded.0[prefix] != 0
    {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    let secret = &decoded.0[prefix + 1..];
    if secret.len() > 1024
        || std::str::from_utf8(secret).map_or(true, |s| s.chars().any(char::is_control))
    {
        return Err(fail(SshFailure::PassphraseUnavailable));
    }
    Ok(SecretBytes(secret.to_vec()))
}

pub(super) fn credential_path(p: &SshProfile) -> PathBuf {
    secret_path(p)
}
