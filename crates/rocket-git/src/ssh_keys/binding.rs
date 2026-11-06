use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
pub struct SshBindingReview {
    pub authentication: SshAuthentication,
    pub profile: Option<SshProfile>,
    pub restoring: bool,
    pub backup: Option<PathBuf>,
    config: Vec<u8>,
    result: Vec<u8>,
    common: PathBuf,
}

pub(super) fn check_service_overrides(
    runner: &Runner,
    cwd: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    // Include effective global/system/worktree configuration as well as local
    // config. Even an empty or currently harmless override is not a fixed service.
    let output = runner.run(
        cwd,
        &args(&[
            "config",
            "--includes",
            "--get-regexp",
            "^remote\\..*\\.(uploadpack|receivepack|vcs)$",
        ]),
        Vec::new(),
        cancel,
    )?;
    match output.status.code() {
        Some(1) => Ok(()),
        Some(0) => Err(fail(SshFailure::Overrides)),
        _ => Err(fail(SshFailure::ToolFailed)),
    }
}

impl Repository {
    fn ssh_binding_compatible(&self, cancel: &CancellationToken) -> Result<()> {
        check_service_overrides(&self.runner, self.cwd(), cancel)?;
        if ["GIT_SSH", "GIT_SSH_COMMAND", "GIT_SSH_VARIANT"]
            .iter()
            .any(|k| std::env::var_os(k).is_some())
        {
            return Err(fail(SshFailure::Overrides));
        }
        let includes = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--local",
                "--no-includes",
                "--get-regexp",
                "^include.*\\.path$",
            ]),
            vec![],
            cancel,
        )?;
        if includes.status.success() {
            return Err(fail(SshFailure::Overrides));
        }
        if includes.status.code() != Some(1) {
            return Err(fail(SshFailure::ToolFailed));
        }
        let worktree = self.runner.run(
            self.cwd(),
            &args(&["config", "--bool", "--get", "extensions.worktreeConfig"]),
            vec![],
            cancel,
        )?;
        if worktree.status.success() && worktree.stdout.starts_with(b"true") {
            return Err(fail(SshFailure::Overrides));
        }
        if !worktree.status.success() && worktree.status.code() != Some(1) {
            return Err(fail(SshFailure::ToolFailed));
        }
        Ok(())
    }
    pub fn preview_ssh_binding(
        &self,
        manager: &SshManager,
        profile: Option<&SshProfile>,
        cancel: &CancellationToken,
    ) -> Result<SshBindingReview> {
        self.preview_ssh_authentication(manager, profile, SshAuthentication::Agent, cancel)
    }
    pub fn preview_ssh_authentication(
        &self,
        manager: &SshManager,
        profile: Option<&SshProfile>,
        authentication: SshAuthentication,
        cancel: &CancellationToken,
    ) -> Result<SshBindingReview> {
        self.runner
            .step("ssh-binding-review", "review_ssh_binding", || {
                cancel.check()?;
                let common = self.info().common_dir.clone();
                let _pins = files::pin(&common)?;
                let config = local_files::read_config(&common.join("config"))?;
                if let Some(p) = profile {
                    self.ssh_binding_compatible(cancel)?;
                    manager.check_config(p, cancel)?;
                    let command = if authentication == SshAuthentication::Direct {
                        if p.encrypted {
                            direct::read_secret(p, &direct::credential_path(p))?;
                        }
                        format!(
                            "'{}' --profile '{}'",
                            git_path(&direct::helper()?)
                                .to_string_lossy()
                                .replace('\\', "/"),
                            git_path(p.config.parent().unwrap())
                                .to_string_lossy()
                                .replace('\\', "/")
                        )
                    } else {
                        format!(
                            "'{}' -F '{}'",
                            git_path(&openssh("ssh.exe")?)
                                .to_string_lossy()
                                .replace('\\', "/"),
                            git_path(&p.config).to_string_lossy().replace('\\', "/")
                        )
                    };
                    // Git parses and updates a private copy. No includes are followed.
                    let folder = common.join("rocket-ssh-recovery");
                    files::ensure_private_parent(&folder)?;
                    let stamp = unique();
                    let scratch = folder.join(format!("review-{stamp}"));
                    files::create_file(&scratch, &config)?;
                    let mut owned = OwnedFile(scratch.clone());
                    let mut a = args(&["config", "--file"]);
                    a.push(git_path(&scratch));
                    a.extend(args(&["--no-includes", "--replace-all", "core.sshCommand"]));
                    a.push(command.into());
                    self.runner
                        .write(self.cwd(), &a, vec![], cancel)?
                        .success("prepare SSH profile binding")?;
                    let result = local_files::read_config(&scratch)?;
                    owned.remove();
                    if local_files::read_config(&common.join("config"))? != config {
                        return Err(fail(SshFailure::Changed));
                    }
                    Ok(SshBindingReview {
                        authentication,
                        profile: Some(p.clone()),
                        restoring: false,
                        backup: None,
                        config,
                        result,
                        common,
                    })
                } else {
                    let folder = common.join("rocket-ssh-recovery");
                    if !folder.exists() {
                        return Err(fail(SshFailure::NoRecovery));
                    }
                    let _pin = files::pin(&folder)?;
                    let mut entries = fs::read_dir(&folder)
                        .map_err(|e| Error::io("read SSH recovery", e))?
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .take(257)
                        .collect::<Vec<_>>();
                    if entries.len() > 256 {
                        return Err(fail(SshFailure::NoRecovery));
                    }
                    entries.sort();
                    entries.reverse();
                    for entry in entries {
                        cancel.check()?;
                        if files::pin(&entry).is_err() {
                            continue;
                        }
                        if read(&entry.join("after"), 2 * 1024 * 1024).is_ok_and(|b| b == config) {
                            let before = read(&entry.join("displaced"), 2 * 1024 * 1024)?;
                            return Ok(SshBindingReview {
                                authentication,
                                profile: None,
                                restoring: true,
                                backup: Some(entry),
                                config,
                                result: before,
                                common,
                            });
                        }
                    }
                    Err(fail(SshFailure::NoRecovery))
                }
            })
    }
    pub fn apply_ssh_binding(
        &self,
        manager: &SshManager,
        review: &SshBindingReview,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("ssh-binding-save", "save_ssh_binding", || {
                cancel.check()?;
                if self.info().common_dir != review.common {
                    return Err(fail(SshFailure::Changed));
                }
                let common = &review.common;
                let _parents = files::pin(common)?;
                if review.profile.is_some() {
                    self.ssh_binding_compatible(cancel)?;
                }
                let _key = review
                    .profile
                    .as_ref()
                    .map(|p| manager.guard(p, cancel))
                    .transpose()?;
                if review.authentication == SshAuthentication::Direct
                    && let Some(p) = &review.profile
                {
                    direct::helper()?;
                    if p.encrypted {
                        direct::read_secret(p, &direct::credential_path(p))?;
                    }
                }
                let config = common.join("config");
                let lock = config.with_extension("lock");
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&lock)
                    .map_err(|e| {
                        Error::io(
                            "lock repository SSH setting; close other Git writers and retry",
                            e,
                        )
                    })?;
                let mut owned = OwnedFile(lock.clone());
                if local_files::read_config(&config)? != review.config {
                    return Err(fail(SshFailure::Changed));
                }
                file.write_all(&review.result)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| Error::io("prepare SSH binding", e))?;
                drop(file);
                let folder = common.join("rocket-ssh-recovery");
                files::ensure_private_parent(&folder)?;
                let _recovery = files::pin(&folder)?;
                let backup = folder.join(unique());
                create_private_dir(&backup)?;
                files::create_file(&backup.join("after"), &review.result)?;
                // Deny in-place writes through publication; replacement races retain the
                // exact displaced bytes in the atomic Windows backup.
                let held = guard_replace(&config)?;
                if local_files::read_config(&config)? != review.config {
                    return Err(fail(SshFailure::Changed));
                }
                cancel.check()?;
                replace(&config, &lock, &backup.join("displaced"))?;
                owned.0 = PathBuf::new();
                drop(held);
                if read(&backup.join("displaced"), 2 * 1024 * 1024)? != review.config
                    || local_files::read_config(&config)? != review.result
                {
                    return Err(fail(SshFailure::RecoveryRequired));
                }
                // No cancellation error after this commit point: the save completed.
                Ok(backup)
            })
    }
}
struct OwnedFile(PathBuf);
impl OwnedFile {
    fn remove(&mut self) {
        if fs::remove_file(&self.0).is_ok() {
            self.0 = PathBuf::new()
        }
    }
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}
pub(super) fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
fn guard_replace(p: &Path) -> Result<File> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(1 | 4).custom_flags(0x00200000);
    }
    let f = o
        .open(p)
        .map_err(|e| Error::io("guard repository SSH settings", e))?;
    let m = f
        .metadata()
        .map_err(|e| Error::io("inspect repository SSH settings", e))?;
    if !m.is_file() || local_files::redirected(&m) || m.permissions().readonly() {
        return Err(fail(SshFailure::UnsafePath));
    }
    Ok(f)
}
#[cfg(windows)]
fn replace(to: &Path, from: &Path, backup: &Path) -> Result<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            to: *const u16,
            from: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
    }
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let (to, from, backup) = (wide(to), wide(from), wide(backup));
    if unsafe {
        ReplaceFileW(
            to.as_ptr(),
            from.as_ptr(),
            backup.as_ptr(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(Error::io(
            "publish SSH repository setting",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace(_: &Path, _: &Path, _: &Path) -> Result<()> {
    Err(fail(SshFailure::Unavailable))
}
