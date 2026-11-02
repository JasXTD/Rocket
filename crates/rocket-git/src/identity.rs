use crate::{
    local_files,
    runner::{args, git_path},
    *,
};
use std::{ffi::OsString, fs, io::Write, path::PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// Local overrides and effective Git user settings. Does not read credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub local: LocalIdentity,
    pub effective: LocalIdentity,
    /// What applies without the local overrides: the global settings, then the
    /// system ones (with their includes).
    pub inherited: LocalIdentity,
    // Exact observation, including unrelated keys, guards against stale edits.
    config: Vec<u8>,
}

pub struct IdentityWriteReport {
    pub result: Result<WriteOutput>,
    pub identity: Result<RepositoryIdentity>,
}

impl LocalIdentity {
    pub fn validate(&self) -> Result<()> {
        for (key, value) in [("Name", &self.name), ("Email", &self.email)] {
            if let Some(value) = value {
                if value.is_empty()
                    || value.len() > 1024
                    || value.trim() != value
                    || value.chars().any(|c| c.is_control() || "<>".contains(c))
                {
                    return Err(Error::InvalidInput(format!(
                        "{key} must be 1–1024 bytes without surrounding whitespace, control characters or angle brackets"
                    )));
                }
                if key == "Email"
                    && (value.chars().any(char::is_whitespace)
                        || !value.split_once('@').is_some_and(|(a, b)| {
                            !a.is_empty() && !b.is_empty() && !b.contains('@')
                        }))
                {
                    return Err(Error::InvalidInput("Enter an email such as name@example.com, or leave it blank to use inherited settings".into()));
                }
            }
        }
        Ok(())
    }
}

impl Repository {
    pub fn identity(&self, cancel: &CancellationToken) -> Result<RepositoryIdentity> {
        self.runner.step("identity-read", "read_identity", || {
            let path = self.info().common_dir.join("config");
            let before = local_files::read_config(&path)?;
            let local = self.identity_values(&["--local", "--no-includes"], cancel)?;
            let effective = self.identity_values(&[], cancel)?;
            let global = self.identity_values(&["--global", "--includes"], cancel)?;
            let system = self.identity_values(&["--system", "--includes"], cancel)?;
            let inherited = LocalIdentity {
                name: global.name.or(system.name),
                email: global.email.or(system.email),
            };
            if local_files::read_config(&path)? != before {
                return Err(Error::InvalidInput(
                    "Repository settings changed while being read; reopen Repository Settings"
                        .into(),
                ));
            }
            Ok(RepositoryIdentity {
                local,
                effective,
                inherited,
                config: before,
            })
        })
    }

    /// `user.name` and `user.email` from the files `scope` selects (no flags: all).
    fn identity_values(&self, scope: &[&str], cancel: &CancellationToken) -> Result<LocalIdentity> {
        let mut arguments = args(&["config"]);
        arguments.extend(args(scope));
        arguments.extend(args(&["--null", "--get-regexp", "^user\\.(name|email)$"]));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(LocalIdentity::default());
        }
        let bytes = output.success("read repository identity")?;
        let mut identity = LocalIdentity::default();
        for entry in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let separator = entry
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| Error::parse("identity", "missing config value"))?;
            let (key, value) = (&entry[..separator], &entry[separator + 1..]);
            let value = String::from_utf8(value.to_vec()).map_err(|_| {
                Error::Unsupported("Non-UTF-8 identity settings must be edited with Git".into())
            })?;
            match key {
                b"user.name" => identity.name = Some(value),
                b"user.email" => identity.email = Some(value),
                _ => {}
            }
        }
        Ok(identity)
    }

    /// Replace both local overrides atomically under Git's config.lock. Blank
    /// overrides are removed; includes, worktree and global files are never edited.
    pub fn set_identity(
        &self,
        expected: &RepositoryIdentity,
        identity: &LocalIdentity,
        cancel: &CancellationToken,
    ) -> Result<IdentityWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("identity-save", "validate_identity", || identity.validate())?;
        let result = self.runner.observe("write_result", "identity-save", || {
            let mut edits = Vec::new();
            for (key, value, existed) in [
                ("user.name", &identity.name, expected.local.name.is_some()),
                (
                    "user.email",
                    &identity.email,
                    expected.local.email.is_some(),
                ),
            ] {
                if value.is_none() && !existed {
                    continue;
                }
                let mut edit = args(&[
                    if value.is_some() {
                        "--replace-all"
                    } else {
                        "--unset-all"
                    },
                    key,
                ]);
                if let Some(value) = value {
                    edit.push(value.into());
                }
                edits.push(edit);
            }
            self.rewrite_local_config(
                &expected.config,
                || {
                    if self.identity(cancel)? != *expected {
                        return Err(Error::InvalidInput(
                            "Repository settings changed since this dialog opened; reopen it before saving"
                                .into(),
                        ));
                    }
                    Ok(())
                },
                &edits,
                cancel,
            )
        });
        let identity = self.runner.observe("refresh", "identity-read", || {
            self.identity(&CancellationToken::new())
        });
        Ok(IdentityWriteReport { result, identity })
    }

    /// Rewrite the repository's own config file through Git's config.lock: take
    /// the lock, let `revalidate` re-check the caller's observation, apply each
    /// edit (arguments after `git config --file <lock> --no-includes`) to a copy
    /// of the `observed` bytes, then publish the copy atomically. Includes,
    /// worktree and global files are never edited; cancellation before the
    /// final rename publishes nothing.
    pub(crate) fn rewrite_local_config(
        &self,
        observed: &[u8],
        revalidate: impl FnOnce() -> Result<()>,
        edits: &[Vec<OsString>],
        cancel: &CancellationToken,
    ) -> Result<WriteOutput> {
        cancel.check()?;
        let config = self.info().common_dir.join("config");
        let lock = config.with_extension("lock");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| {
                Error::io(
                    "lock repository config; close other Git writers and retry",
                    e,
                )
            })?;
        let mut owned = ConfigLock {
            path: lock.clone(),
            armed: true,
        };
        revalidate()?;
        file.write_all(observed)
            .and_then(|_| file.sync_all())
            .map_err(|e| Error::io("prepare repository settings", e))?;
        drop(file);
        for edit in edits {
            let mut arguments = args(&["config", "--file"]);
            arguments.push(git_path(&lock));
            arguments.push("--no-includes".into());
            arguments.extend(edit.iter().cloned());
            self.runner
                .write(self.cwd(), &arguments, vec![], cancel)?
                .success("prepare repository settings")?;
        }
        // The lock protects cooperating Git writers. Detect a non-cooperating
        // edit as well rather than overwriting bytes observed after the dialog.
        if local_files::read_config(&config)? != observed {
            return Err(Error::InvalidInput(
                "Repository config changed during save; reopen Repository Settings".into(),
            ));
        }
        local_files::read_config(&lock)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&lock)
            .and_then(|f| f.sync_all())
            .map_err(|e| Error::io("flush repository settings", e))?;
        cancel.check()?;
        local_files::move_path(&lock, &config, true)?;
        owned.armed = false;
        Ok(WriteOutput {
            stdout: vec![],
            stderr: vec![],
        })
    }
}

struct ConfigLock {
    path: PathBuf,
    armed: bool,
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
