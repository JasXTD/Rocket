//! Global identity editing through Git's parser, with an atomic config lock.
use crate::{
    local_files,
    runner::{args, git_path},
    *,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalIdentity {
    pub path: PathBuf,
    /// Direct values in the file Save will edit. Empty fields remove these overrides.
    pub direct: LocalIdentity,
    /// Global values after includes and lower-priority global files, not repo overrides.
    pub effective: LocalIdentity,
    pub origins: Vec<String>,
    observed: Vec<(PathBuf, Option<Vec<u8>>)>,
}

fn observe(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => local_files::read_config(path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io("inspect global Git configuration", e)),
    }
}

impl GitClient {
    /// Git reports its own global paths, including HOME/XDG/GIT_CONFIG_GLOBAL overrides.
    pub fn global_identity(&self, cancel: &CancellationToken) -> Result<GlobalIdentity> {
        self.runner
            .step("global-identity-read", "read_identity", || {
                let cwd =
                    std::env::current_dir().map_err(|e| Error::io("read current folder", e))?;
                let output = self
                    .runner
                    .run(&cwd, &args(&["var", "GIT_CONFIG_GLOBAL"]), vec![], cancel)?
                    .success("locate global Git configuration")?;
                let paths = std::str::from_utf8(&output).map_err(|_| {
                    Error::Unsupported("Global Git paths must be valid Unicode for editing".into())
                })?;
                let mut observed = Vec::new();
                for line in paths.lines() {
                    let path = PathBuf::from(line);
                    if !path.is_absolute() || line.chars().any(char::is_control) {
                        return Err(Error::Unsupported(
                            "Global Git configuration must use absolute normal paths".into(),
                        ));
                    }
                    observed.push((path.clone(), observe(&path)?));
                }
                let path = observed
                    .iter()
                    .find(|(_, bytes)| bytes.is_some())
                    .or_else(|| observed.first())
                    .ok_or_else(|| {
                        Error::Unsupported("Git did not report a global configuration path".into())
                    })?
                    .0
                    .clone();
                let (direct, _) = if observe(&path)?.is_some() {
                    self.global_values(
                        &cwd,
                        &["--file".into(), git_path(&path), "--no-includes".into()],
                        cancel,
                    )?
                } else {
                    (LocalIdentity::default(), vec![])
                };
                let (effective, origins) =
                    self.global_values(&cwd, &args(&["--global", "--includes"]), cancel)?;
                for (path, bytes) in &observed {
                    if observe(path)? != *bytes {
                        return Err(changed());
                    }
                }
                Ok(GlobalIdentity {
                    path,
                    direct,
                    effective,
                    origins,
                    observed,
                })
            })
    }

    fn global_values(
        &self,
        cwd: &Path,
        flags: &[std::ffi::OsString],
        cancel: &CancellationToken,
    ) -> Result<(LocalIdentity, Vec<String>)> {
        let mut command = args(&["config"]);
        command.extend_from_slice(flags);
        command.extend(args(&[
            "--null",
            "--show-origin",
            "--get-regexp",
            "^user\\.(name|email)$",
        ]));
        let output = self.runner.run(cwd, &command, vec![], cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok((LocalIdentity::default(), vec![]));
        }
        let bytes = output.success("read global identity")?;
        let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
        let mut values = LocalIdentity::default();
        let mut origins = Vec::new();
        for pair in fields.strip_suffix(&[&[][..]]).unwrap_or(&fields).chunks(2) {
            if pair.len() != 2 {
                return Err(Error::parse("global identity", "incomplete origin record"));
            }
            let origin = std::str::from_utf8(pair[0]).map_err(|_| {
                Error::Unsupported("Non-Unicode config origins require Git's editor".into())
            })?;
            let record = std::str::from_utf8(pair[1]).map_err(|_| {
                Error::Unsupported("Non-Unicode identity requires Git's editor".into())
            })?;
            let (key, value) = record
                .split_once('\n')
                .ok_or_else(|| Error::parse("global identity", "missing value"))?;
            match key {
                "user.name" => values.name = Some(value.into()),
                "user.email" => values.email = Some(value.into()),
                _ => {}
            }
            let source = format!("{key}: {origin}");
            if !origins.contains(&source) {
                origins.push(source);
            }
        }
        Ok((values, origins))
    }

    /// Writes only the reviewed global file. Includes and repository config stay intact.
    /// A successful return remains a success if cancellation arrives after publication.
    pub fn set_global_identity(
        &self,
        expected: &GlobalIdentity,
        value: &LocalIdentity,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.runner
            .step("global-identity-save", "save_identity", || {
                value.validate()?;
                cancel.check()?;
                let parent = expected.path.parent().ok_or_else(changed)?;
                local_files::directory(parent)?;
                let mut lock_name = expected.path.as_os_str().to_owned();
                lock_name.push(".lock");
                let lock = PathBuf::from(lock_name);
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock)
                    .map_err(|e| {
                        Error::io("lock global Git config; another writer may be active", e)
                    })?;
                struct Lock(PathBuf);
                impl Drop for Lock {
                    fn drop(&mut self) {
                        let _ = fs::remove_file(&self.0);
                    }
                }
                let _owned = Lock(lock.clone());
                if self.global_identity(cancel)? != *expected {
                    return Err(changed());
                }
                let observed = expected
                    .observed
                    .iter()
                    .find(|(p, _)| p == &expected.path)
                    .ok_or_else(changed)?
                    .1
                    .as_deref()
                    .unwrap_or_default();
                file.write_all(observed)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| Error::io("prepare global identity", e))?;
                drop(file);
                let cwd =
                    std::env::current_dir().map_err(|e| Error::io("read current folder", e))?;
                for (key, new, previous) in [
                    ("user.name", &value.name, &expected.direct.name),
                    ("user.email", &value.email, &expected.direct.email),
                ] {
                    if new == previous {
                        continue;
                    }
                    let mut command = args(&["config", "--file"]);
                    command.push(git_path(&lock));
                    command.extend(args(&[
                        "--no-includes",
                        if new.is_some() {
                            "--replace-all"
                        } else {
                            "--unset-all"
                        },
                        key,
                    ]));
                    if let Some(new) = new {
                        command.push(new.into());
                    }
                    self.runner
                        .write(&cwd, &command, vec![], cancel)?
                        .success("prepare global identity")?;
                }
                if self.global_identity(cancel)? != *expected {
                    return Err(changed());
                }
                local_files::read_config(&lock)?;
                fs::OpenOptions::new()
                    .write(true)
                    .open(&lock)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| Error::io("flush global identity", e))?;
                cancel.check()?;
                publish(&lock, &expected.path, observe(&expected.path)?.is_some())?;
                Ok(())
            })
    }
}
/// Existing configuration retains its Windows access controls and attributes.
/// Do not use MoveFileEx(REPLACE_EXISTING), which substitutes the lock's ACL.
fn publish(from: &Path, to: &Path, exists: bool) -> Result<()> {
    #[cfg(windows)]
    if exists {
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
        let (to, from) = (wide(to), wide(from));
        // SAFETY: live NUL-terminated paths; no backup and no ignored metadata errors.
        if unsafe {
            ReplaceFileW(
                to.as_ptr(),
                from.as_ptr(),
                ptr::null(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(Error::io(
                "publish global Git identity",
                std::io::Error::last_os_error(),
            ));
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    if exists {
        let permissions = fs::metadata(to)
            .map_err(|e| Error::io("read global config permissions", e))?
            .permissions();
        fs::set_permissions(from, permissions)
            .map_err(|e| Error::io("preserve global config permissions", e))?;
    }
    local_files::move_path(from, to, exists)
}
fn changed() -> Error {
    Error::InvalidInput("Global Git settings changed since loading. Reload before saving; existing configuration was preserved.".into())
}
