use crate::{
    model::bytes_to_os,
    remote::{branch_refspec, pattern_matches},
    runner::{FALSE_CONFIG_ENV, args, has_error_diagnostics},
    *,
};
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};

impl Repository {
    /// Push one literal local branch to one literal branch on a configured remote.
    /// Supports new branches and normal fast-forward updates, with optional upstream
    /// setup. Defaults, mirror mode, extra tags, push options and recursive submodule
    /// pushes cannot broaden this selection. Multi-URL/group pushes are rejected.
    ///
    /// Unscoped Git applies URL rewriting, transports, credentials, signing and hooks.
    /// MCP's execution scope separately restricts these before the process starts.
    /// Use WriteTask off the UI thread. The shared gate excludes other Rocket writes;
    /// external Git/config changes are not locked. Git resolves the selected source
    /// ref when its push starts. No automatic network retry/fetch follows a push.
    pub fn push(
        &self,
        remote: impl AsRef<[u8]>,
        local_branch: impl AsRef<[u8]>,
        remote_branch: impl AsRef<[u8]>,
        options: PushOptions,
        cancel: &CancellationToken,
    ) -> Result<PushReport> {
        let _permit = self.acquire_write(cancel)?;
        self.push_locked(remote.as_ref(), local_branch.as_ref(), remote_branch.as_ref(), options, None, cancel)
    }

    /// Caller holds the repository write permit. Publishing additionally pins the
    /// source and URL, and requires the destination branch to remain absent.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_locked(
        &self, remote: &[u8], local_branch: &[u8], remote_branch: &[u8],
        options: PushOptions, new_branch: Option<(&ObjectId, &[u8])>, cancel: &CancellationToken,
    ) -> Result<PushReport> {
        self.push_guarded_locked(remote, local_branch, remote_branch, options, new_branch, None, cancel)
    }

    /// An optional source/destination guard pins a composed commit-and-push
    /// without changing ordinary non-force push semantics.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_guarded_locked(
        &self, remote: &[u8], local_branch: &[u8], remote_branch: &[u8],
        options: PushOptions, new_branch: Option<(&ObjectId, &[u8])>,
        guard: Option<(&ObjectId, &[u8])>, cancel: &CancellationToken,
    ) -> Result<PushReport> {
        let (prefix, source, destination) =
            self.runner.step("push", "validate_selection", || {
                cancel.check()?;
                if remote.is_empty()
                    || remote.len() > 4096
                    || remote.contains(&0)
                    || remote.starts_with(b"-")
                {
                    return Err(Error::InvalidInput(
                        "select one configured remote name without NUL or a leading dash".into(),
                    ));
                }
                let mut prefix = b"refs/remotes/".to_vec();
                prefix.extend_from_slice(remote);
                prefix.push(b'/');
                let mut probe = prefix.clone();
                probe.extend_from_slice(b"rocket-name-check");
                self.check_fetch_ref(&probe, false, cancel)?;
                self.branch_argument(local_branch, cancel)?;
                self.branch_argument(remote_branch, cancel)?;
                let full = |name: &[u8]| [b"refs/heads/".as_slice(), name].concat();
                Ok((prefix, full(local_branch), full(remote_branch)))
            })?;
        let (mut arguments, pinned_source) = self
            .runner
            .step("push", "validate_destination_and_tracking", || {
                self.prepare_push(remote, &prefix, &source, &destination, options, cancel)
            })?;
        if let Some((expected, url)) = new_branch.or(guard) {
            if expected != &pinned_source {
                return Err(Error::InvalidInput(if new_branch.is_some(){"publish_stale: the selected branch changed; review again before pushing."}else{"commit_push_stale: the committed branch changed; local history retained. Review Push separately."}.into()));
            }
            let mut query = args(&["remote", "get-url", "--push", "--all", "--"]);
            query.push(bytes_to_os(remote)?);
            let actual = self.runner.run(self.cwd(), &query, vec![], cancel)?.success("publish destination")?;
            if actual.strip_suffix(b"\n").unwrap_or(&actual) != url {
                return Err(Error::InvalidInput(if new_branch.is_some(){"publish_destination_changed: the effective push URL differs from the reviewed GitHub destination."}else{"commit_push_destination_changed: the effective push URL changed. Local commit retained; review Push separately."}.into()));
            }
            if new_branch.is_some() {
                let at = arguments.iter().position(|a| a == "--").expect("explicit push separator");
                arguments.insert(at, bytes_to_os(&[b"--force-with-lease=".as_slice(), &destination, b":"].concat())?);
            }
        }
        cancel.check()?;
        let started = AtomicBool::new(false);
        let result = self.runner.observe("write_result", "push", || {
            let output = self
                .runner
                .tracked_write(self.cwd(), &arguments, cancel, &started)?;
            if output.status.success() {
                Ok(WriteOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            } else {
                Err(Error::Git {
                    operation: "push",
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
        });
        let stdout = match &result {
            Ok(output) => Some(output.stdout.as_slice()),
            Err(Error::Git { stdout, .. }) if !stdout.is_empty() => Some(stdout.as_slice()),
            _ => None,
        };
        let update = stdout.map(|bytes| {
            self.runner.observe("parse", "push", || {
                let mut update = parse_update(bytes, pinned_source.as_str().as_bytes(), &destination, false)?;
                update.local_reference = source.clone();
                Ok(update)
            })
        });
        let remote_outcome = if !started.load(Ordering::Acquire) {
            PushRemoteOutcome::NotAttempted
        } else {
            remote_outcome(
                result.is_ok(),
                update.as_ref().and_then(|r| r.as_ref().ok()),
            )
        };
        let mut error_diagnostics = match &result {
            Ok(output) => has_error_diagnostics(&output.stderr),
            Err(Error::Git { stderr, .. }) => has_error_diagnostics(stderr),
            _ => false,
        };
        let upstream = (options.set_upstream && remote_outcome == PushRemoteOutcome::Confirmed)
            .then(|| {
                self.runner.observe("refresh", "upstream", || {
                    // An object-ID refspec binds the push to the scanned bytes.
                    // Git cannot infer a local branch from it, so apply the reviewed
                    // upstream only after a confirmed remote update, then verify it.
                    let tracking = [prefix.as_slice(), remote_branch].concat();
                    let mut arguments = args(&["branch"]);
                    arguments.push(bytes_to_os(&[b"--set-upstream-to=".as_slice(), &tracking].concat())?);
                    arguments.push("--".into());
                    arguments.push(bytes_to_os(local_branch)?);
                    self.runner.write(self.cwd(), &arguments, vec![], &CancellationToken::new())?
                        .success("push upstream")?;
                    self.verify_push_upstream(remote, local_branch, &destination)
                })
            });
        if let Some(Err(Error::Git { stderr, .. })) = &upstream {
            error_diagnostics |= has_error_diagnostics(stderr);
        }
        if let Some(log) = &self.runner.log {
            log.push_outcome(remote_outcome, options.set_upstream, error_diagnostics);
        }
        let (branches, status) = self.refresh_after_push();
        Ok(PushReport {
            result,
            update,
            remote_outcome,
            error_diagnostics,
            upstream,
            status,
            branches,
        })
    }

    fn refresh_after_push(&self) -> (Result<Vec<Branch>>, Option<Result<Status>>) {
        // The write is already stopped/reaped. Read both observations afresh,
        // even after cancellation: it does not roll back an accepted push.
        // Neither read depends on the other, and both disable optional index
        // writes. Keep the write permit until this scope has joined so another
        // Rocket write cannot interleave with reconciliation. External Git and
        // file edits can still race; these observations are not an atomic snapshot.
        std::thread::scope(|scope| {
            let branches = std::thread::Builder::new()
                .name("rocket-push-branches".into())
                .spawn_scoped(scope, || {
                    self.runner.observe("refresh", "branches", || {
                        self.branches(&CancellationToken::new())
                    })
                });
            let status = self.info().worktree.as_ref().map(|_| {
                self.runner.observe("refresh", "status", || {
                    self.status(&CancellationToken::new())
                })
            });
            let branches = match branches {
                Ok(worker) => worker.join().unwrap_or(Err(Error::WorkerPanicked)),
                Err(error) => self.runner.observe("refresh", "branches", || {
                    Err(Error::io("start branch refresh worker", error))
                }),
            };
            (branches, status)
        })
    }

    fn prepare_push(
        &self,
        name: &[u8],
        prefix: &[u8],
        source: &[u8],
        destination: &[u8],
        options: PushOptions,
        cancel: &CancellationToken,
    ) -> Result<(Vec<OsString>, ObjectId)> {
        let remote = self
            .remotes(cancel)?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| {
                Error::InvalidInput(
                    "remote is not configured; select a name returned by remotes()".into(),
                )
            })?;
        self.ensure_no_remote_group(name, cancel)?;
        let urls = if remote.push_urls.is_empty() {
            &remote.urls
        } else {
            &remote.push_urls
        };
        if urls.len() != 1
            || urls[0].is_empty()
            || urls[0].iter().any(|b| matches!(b, 0 | b'\n' | b'\r'))
        {
            return Err(Error::Unsupported("push requires exactly one nonempty push URL (or one fetch URL when no push URL is set), without reset entries or line breaks".into()));
        }
        let mut resolve_url = args(&["remote", "get-url", "--push", "--all", "--"]);
        resolve_url.push(bytes_to_os(name)?);
        let effective = self
            .runner
            .run(self.cwd(), &resolve_url, Vec::new(), cancel)?
            .success("resolve push destination")?;
        let effective = effective.strip_suffix(b"\n").unwrap_or(&effective);
        if effective.is_empty() || effective.contains(&b'\n') || effective.contains(&b'\r') {
            return Err(Error::Unsupported(
                "push requires exactly one effective destination after URL rewriting".into(),
            ));
        }
        self.ensure_direct_push_ref(
            source,
            "selected local branch is symbolic; select a direct local branch",
            cancel,
        )?;
        let mut verify = args(&["rev-parse", "--verify", "--quiet", "--end-of-options"]);
        verify.push(bytes_to_os(&[source, b"^{commit}"].concat())?);
        let output = self.runner.run(self.cwd(), &verify, Vec::new(), cancel)?;
        let pinned_source = match output.status.code() {
            Some(0) => {
                crate::parse::oid(crate::parse::line(&output.stdout), "push source")?
            }
            Some(1) => {
                return Err(Error::InvalidInput(
                    "the selected local branch must exist and contain a commit before pushing"
                        .into(),
                ));
            }
            _ => {
                output.success("validate push source")?;
                return Err(Error::parse("push source", "missing source object"));
            }
        };
        if remote.fetch_refspecs.len() > 128 {
            return Err(Error::Unsupported(
                "push supports at most 128 branch tracking mappings".into(),
            ));
        }
        let mut mapped = false;
        let mut excluded = false;
        for spec in &remote.fetch_refspecs {
            let (from, to) = branch_refspec(spec, prefix)?;
            self.check_fetch_ref(from, true, cancel)?;
            if pattern_matches(from, destination) {
                if to.is_some() {
                    mapped = true;
                } else {
                    excluded = true;
                }
            }
        }
        if options.set_upstream && (!mapped || excluded) {
            return Err(Error::Unsupported("upstream setup requires an included fetch mapping for the selected destination branch".into()));
        }
        if mapped {
            // Git push updates tracking refs too; even a wildcard mapping can
            // dereference a symref into a local branch. Guard before transport.
            let tracking = [prefix, &destination[b"refs/heads/".len()..]].concat();
            self.ensure_direct_push_ref(
                &tracking,
                "push tracking destination is symbolic; refusing to update its target",
                cancel,
            )?;
        }
        let mut mirror_override = b"--config-env=remote.".to_vec();
        mirror_override.extend_from_slice(name);
        mirror_override.extend_from_slice(format!(".mirror={FALSE_CONFIG_ENV}").as_bytes());
        let mut arguments = vec![bytes_to_os(&mirror_override)?];
        // --no-mirror alone does NOT override remote.<name>.mirror: Git reapplies
        // that config after option parsing. Override it using a child-only value.
        arguments.extend(args(&[
            "-c",
            "push.autoSetupRemote=false",
            "-c",
            "push.negotiate=false",
            "-c",
            "push.pushOption=",
            "push",
            "--porcelain",
            "--verbose",
            "--no-force",
            "--no-force-with-lease",
            "--no-force-if-includes",
            "--no-follow-tags",
            "--no-prune",
            "--recurse-submodules=check",
        ]));
        arguments.push("--".into());
        arguments.push(bytes_to_os(name)?);
        arguments.push(bytes_to_os(&[pinned_source.as_str().as_bytes(), b":", destination].concat())?);
        Ok((arguments, pinned_source))
    }

    fn ensure_direct_push_ref(
        &self,
        reference: &[u8],
        message: &'static str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut arguments = args(&["symbolic-ref", "--quiet"]);
        arguments.push(bytes_to_os(reference)?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        match output.status.code() {
            Some(1) => Ok(()),
            Some(0) => Err(Error::Unsupported(message.into())),
            _ => output.success("check push reference").map(|_| ()),
        }
    }

    fn verify_push_upstream(&self, remote: &[u8], branch: &[u8], destination: &[u8]) -> Result<()> {
        for (field, expected) in [
            (b".remote".as_slice(), remote),
            (b".merge".as_slice(), destination),
        ] {
            let key = [b"branch.".as_slice(), branch, field].concat();
            let mut arguments = args(&["config", "--null", "--get"]);
            arguments.push(bytes_to_os(&key)?);
            let output = self.runner.run(
                self.cwd(),
                &arguments,
                Vec::new(),
                &CancellationToken::new(),
            )?;
            if !output.status.success() || output.stdout.strip_suffix(b"\0") != Some(expected) {
                return Err(Error::parse(
                    "push upstream",
                    "remote success was reported, but the requested upstream configuration was not verified; inspect branch configuration before retrying",
                ));
            }
        }
        Ok(())
    }
}

fn remote_outcome(success: bool, update: Option<&PushUpdate>) -> PushRemoteOutcome {
    if success {
        return PushRemoteOutcome::Confirmed;
    }
    match update.map(|u| u.kind) {
        Some(PushUpdateKind::New | PushUpdateKind::FastForward | PushUpdateKind::UpToDate) => {
            PushRemoteOutcome::Confirmed
        }
        Some(PushUpdateKind::Rejected(PushRejection::RemoteFailure)) | None => {
            PushRemoteOutcome::Unknown
        }
        Some(PushUpdateKind::Rejected(_)) => PushRemoteOutcome::Rejected,
    }
}

fn parse_update(
    bytes: &[u8],
    source: &[u8],
    destination: &[u8],
    upstream: bool,
) -> Result<PushUpdate> {
    let invalid = || {
        Error::parse(
            "push",
            "missing, unexpected, or malformed porcelain status for the selected branch",
        )
    };
    let lines = bytes.strip_suffix(b"\n").ok_or_else(invalid)?;
    let mut found = None;
    for line in lines.split(|b| *b == b'\n') {
        if line.starts_with(b"To ")
            || line == b"Done"
            || (upstream && line.starts_with(b"branch '") && line.ends_with(b"'."))
        {
            continue;
        }
        if line.get(1) != Some(&b'\t') || found.is_some() {
            return Err(invalid());
        }
        let mut fields = line[2..].splitn(2, |b| *b == b'\t');
        let refs = fields.next().ok_or_else(invalid)?;
        let summary = fields.next().ok_or_else(invalid)?;
        if refs != [source, b":", destination].concat() || summary.is_empty() {
            return Err(invalid());
        }
        let kind = match line[0] {
            b'*' if summary == b"[new branch]" => PushUpdateKind::New,
            b'=' if summary == b"[up to date]" => PushUpdateKind::UpToDate,
            b' ' if fast_forward_summary(summary) => PushUpdateKind::FastForward,
            b'!' if summary.starts_with(b"[rejected]")
                || summary.starts_with(b"[remote rejected]")
                || summary.starts_with(b"[remote failure]") =>
            {
                PushUpdateKind::Rejected(crate::push_failure::rejection(summary))
            }
            _ => return Err(invalid()),
        };
        found = Some(PushUpdate {
            local_reference: source.to_vec(),
            remote_reference: destination.to_vec(),
            kind,
        });
    }
    found.ok_or_else(invalid)
}

fn fast_forward_summary(summary: &[u8]) -> bool {
    let Some(index) = summary.windows(2).position(|p| p == b"..") else {
        return false;
    };
    [&summary[..index], &summary[index + 2..]]
        .into_iter()
        .all(|part| (4..=64).contains(&part.len()) && part.iter().all(u8::is_ascii_hexdigit))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(body: &[u8]) -> Result<PushUpdate> {
        parse_update(body, b"refs/heads/main", b"refs/heads/other", true)
    }
    #[test]
    fn porcelain_status_preserves_selection_and_discards_remote_prose() {
        let cases: &[(&[u8], PushUpdateKind)] = &[
            (
                b"*\trefs/heads/main:refs/heads/other\t[new branch]\n",
                PushUpdateKind::New,
            ),
            (
                b"=\trefs/heads/main:refs/heads/other\t[up to date]\n",
                PushUpdateKind::UpToDate,
            ),
            (
                b" \trefs/heads/main:refs/heads/other\tabcdef0..1234567\n",
                PushUpdateKind::FastForward,
            ),
            (
                b"!\trefs/heads/main:refs/heads/other\t[rejected] (fetch first)\n",
                PushUpdateKind::Rejected(PushRejection::FetchFirst),
            ),
            (
                b"!\trefs/heads/main:refs/heads/other\t[remote rejected] (PRIVATE hook reason)\n",
                PushUpdateKind::Rejected(PushRejection::RemoteRejected),
            ),
        ];
        for (line, kind) in cases {
            let output = [
                b"To PRIVATE credential-bearing URL\n".as_slice(),
                line,
                b"branch 'main' set up to track 'PRIVATE/other'.\nDone\n",
            ]
            .concat();
            let update = parse(&output).unwrap();
            assert_eq!(update.kind, *kind);
            assert!(!format!("{update:?}").contains("PRIVATE"));
        }
    }
    #[test]
    fn malformed_or_broadened_status_is_never_silently_accepted() {
        for output in [
            b"".as_slice(), b"Done\n", b"*\trefs/heads/main:refs/heads/other\t[new branch]",
            b"*\trefs/heads/main:refs/heads/wrong\t[new branch]\n",
            b"+\trefs/heads/main:refs/heads/other\tabcd...1234 (forced update)\n",
            b"-\trefs/heads/main:refs/heads/other\t[deleted]\n",
            b" \trefs/heads/main:refs/heads/other\t..\n",
            b" \trefs/heads/main:refs/heads/other\tabcd...1234\n",
            b"*\trefs/heads/main:refs/heads/other\t[new branch]\n*\trefs/heads/main:refs/heads/other\t[new branch]\n",
            b"PRIVATE hook stdout\n*\trefs/heads/main:refs/heads/other\t[new branch]\n",
        ] { assert!(parse(output).is_err(), "{output:?}"); }
        assert_eq!(remote_outcome(true, None), PushRemoteOutcome::Confirmed);
        assert_eq!(remote_outcome(false, None), PushRemoteOutcome::Unknown);
        let mut update =
            parse(b"!\trefs/heads/main:refs/heads/other\t[remote failure] (lost response)\n")
                .unwrap();
        assert_eq!(
            remote_outcome(false, Some(&update)),
            PushRemoteOutcome::Unknown
        );
        update.kind = PushUpdateKind::Rejected(PushRejection::RemoteRejected);
        assert_eq!(
            remote_outcome(false, Some(&update)),
            PushRemoteOutcome::Rejected
        );
        update.kind = PushUpdateKind::FastForward;
        assert_eq!(
            remote_outcome(false, Some(&update)),
            PushRemoteOutcome::Confirmed
        );
    }
}
