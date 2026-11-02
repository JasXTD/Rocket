//! Explicit, reviewed remote maintenance. Local pruning is a compare-and-swap
//! ref transaction; server deletion uses an exact expected-old-object lease.
use crate::{model::bytes_to_os, runner::args, *};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObsoleteTrackingRef {
    pub reference: Vec<u8>,
    pub id: ObjectId,
}
#[derive(Clone, Debug)]
pub struct RemoteCleanupPreview {
    repository: PathBuf,
    config: Vec<u8>,
    destination: Vec<u8>,
    remote: Vec<u8>,
    obsolete: Vec<ObsoleteTrackingRef>,
    stale_local: Vec<Vec<u8>>,
}
impl RemoteCleanupPreview {
    pub fn remote(&self) -> &[u8] {
        &self.remote
    }
    pub fn obsolete(&self) -> &[ObsoleteTrackingRef] {
        &self.obsolete
    }
    /// A missing upstream is a candidate for review, not proof a branch is merged.
    pub fn stale_local_branches(&self) -> &[Vec<u8>] {
        &self.stale_local
    }
}
#[derive(Clone, Debug)]
pub struct RemoteBranchDeletePreview {
    repository: PathBuf,
    config: Vec<u8>,
    destination: Vec<u8>,
    remote: Vec<u8>,
    branch: Vec<u8>,
    expected: ObjectId,
}
impl RemoteBranchDeletePreview {
    pub fn remote(&self) -> &[u8] {
        &self.remote
    }
    pub fn branch(&self) -> &[u8] {
        &self.branch
    }
    pub fn expected(&self) -> &ObjectId {
        &self.expected
    }
    pub fn destination(&self) -> &[u8] {
        &self.destination
    }
}
#[derive(Debug)]
pub struct RemoteCleanupReport {
    pub result: Result<()>,
    pub recovery: Option<String>,
    /// None means a failed observation, not confirmed deletion.
    pub remote_deleted: Option<bool>,
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
fn stale() -> Error {
    invalid(
        "remote_cleanup_stale: configuration or refs changed; review again. No cleanup was started.",
    )
}
fn split(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let i = bytes.iter().position(|b| *b == separator)?;
    Some((&bytes[..i], &bytes[i + 1..]))
}
struct Advertised {
    refs: BTreeMap<Vec<u8>, ObjectId>,
    default: Option<Vec<u8>>,
}
impl Repository {
    fn cleanup_config(&self, c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(
                self.cwd(),
                &args(&["config", "--null", "--list"]),
                vec![],
                c,
            )?
            .success("remote-cleanup")
    }
    fn cleanup_destination(&self, remote: &[u8], c: &CancellationToken) -> Result<Vec<u8>> {
        let (prefix, argument) = self.validate_fetch_selection(remote, c)?;
        self.prepare_fetch(remote, &prefix, argument, c)?;
        let configured = self
            .remotes(c)?
            .into_iter()
            .find(|r| r.name == remote)
            .ok_or_else(stale)?;
        let wanted = [b"+refs/heads/".as_slice(), b"*:", &prefix, b"*"].concat();
        if configured.urls.len() != 1 || configured.fetch_refspecs != [wanted] {
            return Err(invalid(
                "remote_cleanup_mapping: cleanup requires one URL and the standard complete branch mapping; custom or negative refspecs are retained.",
            ));
        }
        let mut command = args(&["remote", "get-url", "--all", "--"]);
        command.push(bytes_to_os(remote)?);
        let raw = self
            .runner
            .run(self.cwd(), &command, vec![], c)?
            .success("remote-cleanup")?;
        let destination = parse::line(&raw).to_vec();
        if destination.is_empty()
            || destination.starts_with(b"-")
            || destination.iter().any(|b| matches!(b, 0 | b'\n' | b'\r'))
        {
            return Err(stale());
        }
        Ok(destination)
    }
    fn cleanup_advertisement(
        &self,
        destination: &[u8],
        c: &CancellationToken,
        reconcile: bool,
    ) -> Result<Advertised> {
        let mut runner = self.runner.clone();
        if reconcile {
            runner.limits.timeout = Duration::from_secs(10);
        }
        let mut command = args(&[
            "-c",
            "protocol.version=2",
            "ls-remote",
            "--symref",
            "--server-option=",
            "--",
        ]);
        command.push(bytes_to_os(destination)?);
        command.extend(args(&["HEAD", "refs/heads/*"]));
        let raw = runner
            .run(self.cwd(), &command, vec![], c)?
            .success("remote-cleanup")?;
        let mut refs = BTreeMap::new();
        let mut default = None;
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let (value, name) = split(line, b'\t').ok_or_else(|| {
                invalid("remote_cleanup_response: malformed branch advertisement")
            })?;
            if name == b"HEAD" {
                if let Some(target) = value.strip_prefix(b"ref: ") {
                    default = Some(target.to_vec());
                }
                continue;
            }
            if !name.starts_with(b"refs/heads/")
                || value.starts_with(b"ref: ")
                || refs.len() >= 10_000
            {
                return Err(invalid(
                    "remote_cleanup_response: indirect, unexpected or excessive remote branches",
                ));
            }
            let id = parse::oid(value, "remote-cleanup")?;
            if refs.insert(name.to_vec(), id).is_some() {
                return Err(invalid(
                    "remote_cleanup_response: duplicate advertised branch",
                ));
            }
        }
        Ok(Advertised { refs, default })
    }
    fn cleanup_tracking(
        &self,
        remote: &[u8],
        c: &CancellationToken,
    ) -> Result<Vec<ObsoleteTrackingRef>> {
        let prefix = [b"refs/remotes/".as_slice(), remote, b"/"].concat();
        let mut command = args(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)",
            "--",
        ]);
        command.push(bytes_to_os(&prefix)?);
        let raw = self
            .runner
            .run(self.cwd(), &command, vec![], c)?
            .success("remote-cleanup")?;
        let mut out = Vec::new();
        for row in raw.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
            let fields = row.split(|b| *b == 0).collect::<Vec<_>>();
            if fields.len() != 3 || !fields[0].starts_with(&prefix) || out.len() >= 10_000 {
                return Err(invalid(
                    "remote_cleanup_response: unsupported tracking refs",
                ));
            }
            if fields[2].is_empty() {
                out.push(ObsoleteTrackingRef {
                    reference: fields[0].to_vec(),
                    id: parse::oid(fields[1], "remote-cleanup")?,
                });
            }
        }
        Ok(out)
    }
    pub fn preview_remote_cleanup(
        &self,
        remote: &[u8],
        c: &CancellationToken,
    ) -> Result<RemoteCleanupPreview> {
        self.runner
            .step("remote-cleanup-preview", "review_remote_cleanup", || {
                let config = self.cleanup_config(c)?;
                let destination = self.cleanup_destination(remote, c)?;
                let tracking = self.cleanup_tracking(remote, c)?;
                let advertised = self.cleanup_advertisement(&destination, c, false)?;
                let prefix = [b"refs/remotes/".as_slice(), remote, b"/"].concat();
                let obsolete = tracking
                    .iter()
                    .filter(|r| {
                        !advertised.refs.contains_key(
                            &[b"refs/heads/".as_slice(), &r.reference[prefix.len()..]].concat(),
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let local = self
                    .runner
                    .run(
                        self.cwd(),
                        &args(&[
                            "for-each-ref",
                            "--format=%(refname:short)%00%(upstream)",
                            "refs/heads/",
                        ]),
                        vec![],
                        c,
                    )?
                    .success("remote-cleanup")?;
                let stale_local = local
                    .split(|b| *b == b'\n')
                    .filter_map(|row| {
                        let (name, upstream) = split(row, 0)?;
                        (upstream.starts_with(&prefix)
                            && (!tracking.iter().any(|r| r.reference == upstream)
                                || obsolete.iter().any(|r| r.reference == upstream)))
                        .then(|| name.to_vec())
                    })
                    .collect();
                if config != self.cleanup_config(c)?
                    || tracking != self.cleanup_tracking(remote, c)?
                    || destination != self.cleanup_destination(remote, c)?
                {
                    return Err(stale());
                }
                Ok(RemoteCleanupPreview {
                    repository: self.info().common_dir.clone(),
                    config,
                    destination,
                    remote: remote.to_vec(),
                    obsolete,
                    stale_local,
                })
            })
    }
    pub fn prune_tracking_refs(
        &self,
        p: &RemoteCleanupPreview,
        selected: &[Vec<u8>],
        c: &CancellationToken,
    ) -> Result<RemoteCleanupReport> {
        let _permit = self.acquire_write(c)?;
        self.runner
            .step("remote-prune", "revalidate_remote_cleanup", || {
                if p.repository != self.info().common_dir
                    || p.config != self.cleanup_config(c)?
                    || p.destination != self.cleanup_destination(&p.remote, c)?
                {
                    return Err(stale());
                }
                if selected.is_empty()
                    || selected.len() > 1000
                    || (1..selected.len()).any(|i| selected[..i].contains(&selected[i]))
                {
                    return Err(invalid("Select 1–1000 distinct obsolete tracking refs."));
                }
                let current = self.preview_remote_cleanup(&p.remote, c)?;
                for name in selected {
                    let old = p
                        .obsolete
                        .iter()
                        .find(|r| &r.reference == name)
                        .ok_or_else(stale)?;
                    if !current.obsolete.contains(old) {
                        return Err(stale());
                    }
                }
                Ok(())
            })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("System clock is before the Unix epoch"))?
            .as_nanos();
        let recovery = format!("refs/rocket/remote-prune/{}-{stamp}", std::process::id());
        let mut input = b"start\noption no-deref\n".to_vec();
        for (i, name) in selected.iter().enumerate() {
            let entry = p
                .obsolete
                .iter()
                .find(|r| &r.reference == name)
                .ok_or_else(stale)?;
            input.extend_from_slice(
                format!("create {recovery}/{i} {}\ndelete ", entry.id).as_bytes(),
            );
            input.extend_from_slice(name);
            input.extend_from_slice(format!(" {}\n", entry.id).as_bytes());
        }
        input.extend_from_slice(b"prepare\ncommit\n");
        let result = self
            .runner
            .step("remote-prune", "prune_selected_tracking_refs", || {
                self.runner
                    .write(self.cwd(), &args(&["update-ref", "--stdin"]), input, c)?
                    .success("remote-prune")
                    .map(|_| ())
            });
        Ok(RemoteCleanupReport {
            result,
            recovery: Some(recovery),
            remote_deleted: None,
        })
    }
    pub fn preview_remote_branch_delete(
        &self,
        remote: &[u8],
        branch: &[u8],
        c: &CancellationToken,
    ) -> Result<RemoteBranchDeletePreview> {
        self.runner.step(
            "remote-branch-delete-preview",
            "review_remote_branch_delete",
            || {
                let config = self.cleanup_config(c)?;
                let destination = self.reviewed_push_destination(remote, c)?;
                let reference = [b"refs/heads/".as_slice(), branch].concat();
                let mut command = args(&["check-ref-format"]);
                command.push(bytes_to_os(&reference)?);
                self.runner
                    .run(self.cwd(), &command, vec![], c)?
                    .success("remote-branch-delete-preview")?;
                let advertised = self.cleanup_advertisement(&destination, c, false)?;
                let default = advertised.default.as_deref().ok_or_else(|| invalid(
                    "remote_branch_default: the server did not identify its default branch. Deletion is refused until that can be checked.",
                ))?;
                if default == reference.as_slice() {
                    return Err(invalid(
                        "remote_branch_default: deleting the remote default branch is refused.",
                    ));
                }
                let expected = advertised.refs.get(&reference).cloned().ok_or_else(|| {
                    invalid("remote_branch_missing: refresh and select an existing remote branch.")
                })?;
                if config != self.cleanup_config(c)? {
                    return Err(stale());
                }
                Ok(RemoteBranchDeletePreview {
                    repository: self.info().common_dir.clone(),
                    config,
                    destination,
                    remote: remote.to_vec(),
                    branch: branch.to_vec(),
                    expected,
                })
            },
        )
    }
    pub fn delete_remote_branch(
        &self,
        p: &RemoteBranchDeletePreview,
        c: &CancellationToken,
    ) -> Result<RemoteCleanupReport> {
        let _permit = self.acquire_write(c)?;
        self.runner.step("remote-branch-delete","revalidate_remote_branch_delete",|| {
            if p.repository!=self.info().common_dir || p.config!=self.cleanup_config(c)? || p.destination!=self.reviewed_push_destination(&p.remote,c)? { return Err(stale()); }
            let fresh=self.preview_remote_branch_delete(&p.remote,&p.branch,c)?;
            if fresh.expected!=p.expected { return Err(invalid("remote_branch_changed: the server branch changed after review. Nothing was deleted.")); } Ok(())
        })?;
        let reference = [b"refs/heads/".as_slice(), &p.branch].concat();
        let mut command = args(&[
            "-c",
            "push.autoSetupRemote=false",
            "-c",
            "push.negotiate=false",
            "-c",
            "push.pushOption=",
            "push",
            "--porcelain",
            "--no-force",
            "--no-force-with-lease",
            "--no-force-if-includes",
            "--no-follow-tags",
            "--no-prune",
            "--no-recurse-submodules",
            "--no-mirror",
        ]);
        command.push(bytes_to_os(
            &[
                b"--force-with-lease=".as_slice(),
                &reference,
                b":",
                p.expected.as_str().as_bytes(),
            ]
            .concat(),
        )?);
        command.push("--".into());
        command.push(bytes_to_os(&p.destination)?);
        command.push(bytes_to_os(&[b":".as_slice(), &reference].concat())?);
        let result=self.runner.step("remote-branch-delete","delete_selected_remote_branch",|| {
            let output=self.runner.write(self.cwd(),&command,vec![],c)?;
            if !output.status.success(){return output.success("push").map(|_|());}
            if crate::runner::has_error_diagnostics(&output.stderr){return Err(invalid("remote_branch_diagnostics: Git reported an additional error; inspect the remote result before retrying."));}
            let expected=[b"-\t:".as_slice(),&reference,b"\t[deleted]"].concat();
            let rows=output.stdout.split(|b|*b==b'\n').map(|r|r.strip_suffix(b"\r").unwrap_or(r)).filter(|r|r.contains(&b'\t')).collect::<Vec<_>>();
            if rows!=[expected.as_slice()]{return Err(invalid("remote_branch_response: deletion response was not the exact reviewed branch. Inspect the observed remote state."));}
            Ok(())
        });
        let observed = self.runner.step(
            "remote-branch-delete",
            "observe_remote_branch_delete",
            || {
                let fresh = CancellationToken::new();
                if p.config != self.cleanup_config(&fresh)?
                    || p.destination != self.reviewed_push_destination(&p.remote, &fresh)?
                {
                    return Err(stale());
                }
                self.cleanup_advertisement(&p.destination, &fresh, true)
                    .map(|a| !a.refs.contains_key(&reference))
            },
        );
        Ok(RemoteCleanupReport {
            result,
            recovery: None,
            remote_deleted: observed.ok(),
        })
    }
}
