use crate::{model::bytes_to_os, runner::args, *};
use std::ffi::OsString;
use std::{path::PathBuf, fs::{self, OpenOptions}, io::Write, time::{SystemTime, UNIX_EPOCH}};

#[derive(Clone)]
pub struct RemoteRenamePreview {
    repository: PathBuf,
    remote: Remote,
    name: Vec<u8>,
    config: Vec<u8>,
    effective: Vec<u8>,
    branches: Vec<Branch>,
}
impl std::fmt::Debug for RemoteRenamePreview {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.debug_struct("RemoteRenamePreview").finish_non_exhaustive() }
}
impl RemoteRenamePreview {
    pub fn remote(&self)->&Remote { &self.remote }
    pub fn name(&self)->&[u8] { &self.name }
    pub fn affected_branches(&self)->Vec<&Branch> {
        let prefix=[b"refs/remotes/".as_slice(), &self.remote.name, b"/"].concat();
        self.branches.iter().filter(|b|b.upstream.as_ref().is_some_and(|u|u.starts_with(&prefix))).collect()
    }
}
#[derive(Debug)]
pub struct RemoteRenameReport {
    pub report: RemoteWriteReport,
    pub config_backup: PathBuf,
    pub recovery_refs: String,
}

impl Repository {
    pub fn preview_remote_rename(&self, remote:&Remote, name:&[u8], c:&CancellationToken)->Result<RemoteRenamePreview> {
        self.runner.step("remote-rename-preview","review_remote_rename",||{
            self.validate_managed_remote(remote,c)?;
            self.validate_managed_remote_name(name,c)?;
            if name == remote.name || !self.remote_values(name,false,c)?.is_empty() {
                return Err(Error::InvalidInput("remote_rename_name: choose a new remote name that is not already configured.".into()));
            }
            let prefix=[b"refs/remotes/".as_slice(), &remote.name, b"/"].concat();
            for spec in &remote.fetch_refspecs { crate::remote::branch_refspec(spec,&prefix)?; }
            let config=crate::local_files::read_config(&self.info().common_dir.join("config"))?;
            let effective=self.runner.run(self.cwd(),&args(&["config","--null","--list"]),vec![],c)?.success("remote-rename-preview")?;
            let branches=self.branches(c)?;
            let destination=[b"refs/remotes/".as_slice(),name,b"/"].concat();
            if branches.iter().any(|b|b.reference.starts_with(&destination)) { return Err(Error::InvalidInput("remote_rename_collision: tracking refs already use the new name.".into())); }
            self.validate_managed_remote(remote,c)?;
            if crate::local_files::read_config(&self.info().common_dir.join("config"))?!=config { return Err(Error::InvalidInput("remote_rename_stale: configuration changed during review.".into())); }
            Ok(RemoteRenamePreview { repository:self.info().common_dir.clone(),remote:remote.clone(),name:name.to_vec(),config,effective,branches })
        })
    }
    pub fn rename_remote(&self,p:&RemoteRenamePreview,c:&CancellationToken)->Result<RemoteRenameReport> {
        let _permit=self.acquire_write(c)?;
        let revalidate=||->Result<()> {
            let current=self.preview_remote_rename(&p.remote,&p.name,c)?;
            if p.repository!=current.repository || p.config!=current.config || p.effective!=current.effective || p.branches!=current.branches {
                return Err(Error::InvalidInput("remote_rename_stale: configuration or refs changed; review again. Nothing was renamed.".into()));
            } Ok(())
        };
        self.runner.step("remote-rename","revalidate_remote_rename",revalidate)?;
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|Error::InvalidInput("invalid system clock".into()))?.as_nanos();
        let folder=self.info().common_dir.join(format!("rocket-remote-rename-{}-{stamp}",std::process::id()));
        fs::create_dir(&folder).map_err(|e|Error::io("create remote rename recovery folder",e))?;
        let _pins=crate::local_files::pin_directories(&self.info().common_dir,&folder)?;
        let backup=folder.join("config.before");
        let mut file=OpenOptions::new().write(true).create_new(true).open(&backup).map_err(|e|Error::io("create remote configuration backup",e))?;
        file.write_all(&p.config).and_then(|_|file.sync_all()).map_err(|e|Error::io("write remote configuration backup",e))?;
        let recovery=format!("refs/rocket/remote-rename/{}-{stamp}",std::process::id());
        let prefix=[b"refs/remotes/".as_slice(),&p.remote.name,b"/"].concat();
        let mut input=b"start\n".to_vec();
        for (i,b) in p.branches.iter().filter(|b|b.reference.starts_with(&prefix)&&b.symbolic_target.is_none()).enumerate() {
            input.extend_from_slice(format!("create {recovery}/{i} {}\n",b.target).as_bytes());
        }
        input.extend_from_slice(b"prepare\ncommit\n");
        self.runner.step("remote-rename","backup_remote_tracking",||self.runner.write(self.cwd(),&args(&["update-ref","--no-deref","--stdin"]),input,c)?.success("remote-rename").map(|_|()))?;
        revalidate()?;
        let mut arguments=args(&["remote","rename","--"]);
        arguments.extend([bytes_to_os(&p.remote.name)?,bytes_to_os(&p.name)?]);
        let report=self.remote_write("remote-rename",&arguments,c);
        Ok(RemoteRenameReport {report,config_backup:backup,recovery_refs:recovery})
    }
    /// Add local configuration only. Does not fetch, contact a server, or create
    /// a remote repository. Git supplies the normal branch tracking refspec.
    pub fn add_remote(
        &self,
        name: &[u8],
        url: &[u8],
        cancel: &CancellationToken,
    ) -> Result<RemoteWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner.step("remote-add", "validate_remote", || {
            self.validate_managed_remote_name(name, cancel)?;
            validate_url(url)?;
            if !self.remote_values(name, false, cancel)?.is_empty() {
                return Err(Error::InvalidInput(
                    "a remote with this name already exists; choose another name or edit it".into(),
                ));
            }
            Ok(())
        })?;
        let mut arguments = args(&["remote", "add", "--"]);
        arguments.extend([bytes_to_os(name)?, bytes_to_os(url)?]);
        Ok(self.remote_write("remote-add", &arguments, cancel))
    }

    /// Replace one fetch/push URL. None for Push restores Git's fetch-URL fallback.
    /// Refuse multiple values rather than silently removing extra destinations.
    /// The expected observation guards against changes since opening the editor.
    pub fn set_remote_url(
        &self,
        expected: &Remote,
        kind: RemoteUrlKind,
        url: Option<&[u8]>,
        cancel: &CancellationToken,
    ) -> Result<RemoteWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner.step("remote-edit", "validate_remote", || {
            self.validate_managed_remote(expected, cancel)?;
            let values = match kind { RemoteUrlKind::Fetch => &expected.urls, RemoteUrlKind::Push => &expected.push_urls };
            if values.len() > 1 {
                return Err(Error::Unsupported("this remote has multiple URLs for this direction; edit its configuration with Git to preserve every destination".into()));
            }
            if let Some(url) = url { validate_url(url)?; }
            else if kind == RemoteUrlKind::Fetch {
                return Err(Error::InvalidInput("a fetch URL is required".into()));
            }
            Ok(())
        })?;
        let mut key = b"remote.".to_vec();
        key.extend_from_slice(&expected.name);
        key.extend_from_slice(if kind == RemoteUrlKind::Fetch {
            b".url"
        } else {
            b".pushurl"
        });
        let mut arguments = args(&["config", "--local", "--no-includes"]);
        if url.is_none() {
            if expected.push_urls.is_empty() {
                return Ok(self.remote_observations(Ok(WriteOutput {
                    stdout: vec![],
                    stderr: vec![],
                })));
            }
            arguments.push("--unset".into());
        }
        arguments.push(bytes_to_os(&key)?);
        if let Some(url) = url {
            arguments.push(bytes_to_os(url)?);
        }
        Ok(self.remote_write("remote-edit", &arguments, cancel))
    }

    /// Remove local configuration and tracking refs. Nothing on the server is deleted.
    pub fn remove_remote(
        &self,
        expected: &Remote,
        cancel: &CancellationToken,
    ) -> Result<RemoteWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner.step("remote-remove", "validate_remote", || {
            self.validate_managed_remote(expected, cancel)?;
            let mut prefix = b"refs/remotes/".to_vec();
            prefix.extend_from_slice(&expected.name);
            prefix.push(b'/');
            for spec in &expected.fetch_refspecs {
                // Git interprets fetch mappings during removal. Refuse mappings
                // that could expand deletion outside this tracking namespace.
                crate::remote::branch_refspec(spec, &prefix)?;
            }
            Ok(())
        })?;
        let mut arguments = args(&["remote", "remove", "--"]);
        arguments.push(bytes_to_os(&expected.name)?);
        Ok(self.remote_write("remote-remove", &arguments, cancel))
    }

    pub(crate) fn validate_managed_remote_name(&self, name: &[u8], cancel: &CancellationToken) -> Result<()> {
        if name.is_empty()
            || name.len() > 1024
            || name.starts_with(b"-")
            || name.iter().any(|b| b.is_ascii_control())
        {
            return Err(Error::InvalidInput(
                "remote name must be 1..1024 bytes without control characters or a leading dash"
                    .into(),
            ));
        }
        let mut reference = b"refs/remotes/".to_vec();
        reference.extend_from_slice(name);
        reference.extend_from_slice(b"/rocket-name-check");
        self.check_fetch_ref(&reference, false, cancel)?;
        self.ensure_no_remote_group(name, cancel)
    }

    fn validate_managed_remote(&self, expected: &Remote, cancel: &CancellationToken) -> Result<()> {
        self.validate_managed_remote_name(&expected.name, cancel)?;
        if self
            .remotes(cancel)?
            .iter()
            .find(|r| r.name == expected.name)
            != Some(expected)
        {
            return Err(Error::InvalidInput(
                "remote configuration changed or was removed; refresh and reopen the remote editor"
                    .into(),
            ));
        }
        let effective = self.remote_values(&expected.name, false, cancel)?;
        let local = self.remote_values(&expected.name, true, cancel)?;
        if local.is_empty() || local != effective {
            return Err(Error::Unsupported("this remote has settings from an include, worktree, global or system config; edit it at its source with Git".into()));
        }
        Ok(())
    }

    // Compare every key, including options the Remote model does not expose.
    // Names are compared as bytes, never interpolated into a regex.
    pub(crate) fn remote_values(
        &self,
        name: &[u8],
        local: bool,
        cancel: &CancellationToken,
    ) -> Result<Vec<Vec<u8>>> {
        let mut arguments = args(&["config"]);
        if local {
            arguments.extend(args(&["--local", "--no-includes"]));
        }
        arguments.extend(args(&["--null", "--get-regexp", r"^remote\."]));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(vec![]);
        }
        let output = output.success("read remote configuration")?;
        let mut values = Vec::new();
        for record in output.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let key = record.split(|b| *b == b'\n').next().unwrap();
            if let Some(rest) = key.strip_prefix(b"remote.")
                && let Some(dot) = rest.iter().rposition(|b| *b == b'.')
                && &rest[..dot] == name
            {
                values.push(record.to_vec());
            }
        }
        Ok(values)
    }

    fn remote_write(
        &self,
        operation: &'static str,
        arguments: &[OsString],
        cancel: &CancellationToken,
    ) -> RemoteWriteReport {
        let result = self.runner.observe("write_result", operation, || {
            let output = self.runner.write(self.cwd(), arguments, vec![], cancel)?;
            if !output.status.success() {
                return Err(Error::Git {
                    operation,
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            Ok(WriteOutput {
                stdout: output.stdout,
                stderr: output.stderr,
            })
        });
        self.remote_observations(result)
    }

    fn remote_observations(&self, result: Result<WriteOutput>) -> RemoteWriteReport {
        let cancel = CancellationToken::new();
        RemoteWriteReport {
            result,
            remotes: self
                .runner
                .observe("refresh", "remotes", || self.remotes(&cancel)),
            branches: self
                .runner
                .observe("refresh", "branches", || self.branches(&cancel)),
            status: self.info().worktree.as_ref().map(|_| {
                self.runner
                    .observe("refresh", "status", || self.status(&cancel))
            }),
        }
    }
}

pub(crate) fn validate_url(url: &[u8]) -> Result<()> {
    if url.is_empty()
        || url.len() > 8192
        || url.starts_with(b"-")
        || url.iter().any(|b| b.is_ascii_control())
    {
        return Err(Error::InvalidInput(
            "remote URL/path must be 1..8192 bytes without control characters or a leading dash"
                .into(),
        ));
    }
    Ok(())
}
