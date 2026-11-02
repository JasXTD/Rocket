use super::*;
use std::sync::mpsc::SyncSender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsUploadOutcome {
    NotStarted,
    /// Helper exited successfully. This is not an independent remote read-back.
    ConfirmedByHelper,
    /// Transfer was invoked; cancellation/failure cannot establish which objects arrived.
    Unknown,
}
impl LfsUploadOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::ConfirmedByHelper => "confirmed_by_helper",
            Self::Unknown => "unknown",
        }
    }
}

/// Conservative diagnostic hints, never instructions to retry or weaken authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsPushFailure {
    Transport(FetchFailure),
    Quota,
    Rejected,
    Other,
}
impl LfsPushFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Transport(f) => f.code(),
            Self::Quota => "lfs_upload_quota",
            Self::Rejected => "lfs_upload_rejected",
            Self::Other => "lfs_upload_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Transport(FetchFailure::CredentialsUnavailable) => {
                "Sign in using the configured credential helper outside Rocket, then explicitly review and retry the upload."
            }
            Self::Transport(f) => f.guidance(),
            Self::Quota => {
                "The LFS server reports a storage or bandwidth limit. Check the repository's LFS quota with its administrator before retrying."
            }
            Self::Rejected => {
                "The LFS server rejected an upload request. Check write access, object limits and server policy before retrying."
            }
            Self::Other => {
                "The LFS helper failed without a recognized diagnostic. Inspect the helper/server locally. Do not assume no objects arrived or retry automatically."
            }
        }
    }
    pub(crate) fn classify(stdout: &[u8], stderr: &[u8]) -> Self {
        // Bounded process output is retained in Error::Git for explicit inspection only.
        let mut bytes = stderr.to_vec();
        bytes.push(b'\n');
        bytes.extend_from_slice(stdout);
        let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
        if text.contains("exceeded its data quota")
            || text.contains("is over its data quota")
            || text.contains("storage quota")
            || text.contains("bandwidth quota")
        {
            return Self::Quota;
        }
        if text.contains("x509:") || text.contains("tls: failed to verify certificate") {
            return Self::Transport(FetchFailure::TlsVerificationFailed);
        }
        if text.contains("lookup ") && text.contains("no such host") {
            return Self::Transport(FetchFailure::NameResolutionFailed);
        }
        if text.contains("i/o timeout") || text.contains("tls handshake timeout") {
            return Self::Transport(FetchFailure::ConnectionTimedOut);
        }
        let transport = FetchFailure::classify(&bytes);
        if transport != FetchFailure::Other {
            return Self::Transport(transport);
        }
        if text.contains("authorization error")
            || text.contains("status code: 401")
            || text.contains("status 401")
        {
            return Self::Transport(FetchFailure::AuthenticationRejected);
        }
        if text.contains("status code: 403") || text.contains("status 403") {
            return Self::Transport(FetchFailure::AccessDenied);
        }
        if text.contains("upload rejected")
            || text.contains("status code: 422")
            || text.contains("status code: 413")
        {
            return Self::Rejected;
        }
        Self::Other
    }
}
fn unique(entries: &[LfsEntry]) -> Result<Vec<LfsEntry>> {
    let mut objects: BTreeMap<&str, &LfsEntry> = BTreeMap::new();
    for e in entries {
        if let Some(old) = objects.insert(&e.object.oid, e)
            && old.object.size != e.object.size
        {
            return fail(LfsFailure::InvalidObject);
        }
    }
    if objects.is_empty() {
        return fail(LfsFailure::NoObjects);
    }
    if objects.len() > ENTRY_LIMIT
        || objects.values().any(|e| e.object.size > LFS_FILE_LIMIT)
        || objects
            .values()
            .try_fold(0u64, |n, e| n.checked_add(e.object.size))
            .is_none_or(|n| n > 2 * 1024 * 1024 * 1024)
    {
        return fail(LfsFailure::Limit);
    }
    Ok(objects.into_values().cloned().collect())
}
fn update(sender: Option<&SyncSender<FetchProgress>>, phase: FetchPhase, current: u64, total: u64) {
    if let Some(sender) = sender {
        let _ = sender.try_send(FetchProgress {
            phase,
            current,
            total: Some(total),
        });
    }
}
pub(super) fn validate(
    repo: &Repository,
    helper: &Path,
    entries: &[LfsEntry],
    progress: Option<&SyncSender<FetchProgress>>,
    c: &CancellationToken,
) -> Result<()> {
    let entries = unique(entries)?;
    let total = entries.len() as u64;
    update(progress, FetchPhase::CheckingLfsObjects, 0, total);
    for (i, e) in entries.iter().enumerate() {
        c.check()?;
        disk::verify_cached(repo, helper, &e.object, c)?;
        update(
            progress,
            FetchPhase::CheckingLfsObjects,
            i as u64 + 1,
            total,
        );
    }
    Ok(())
}
pub(super) fn apply(
    repo: &Repository,
    p: &LfsPreview,
    remote: &str,
    progress: Option<&SyncSender<FetchProgress>>,
    c: &CancellationToken,
    outcome: &mut Option<LfsUploadOutcome>,
) -> Result<()> {
    let entries = unique(&p.entries)?;
    // Pin every local payload while hashing and while the helper reads it. On Windows
    // these handles deny both writes (including hard-link writes) and replacement.
    let _objects = disk::pin_source(repo, &entries, c)?;
    repo.runner.step("lfs-push", "validate_lfs_upload", || {
        validate(repo, &p.helper, &entries, progress, c)
    })?;
    let (destination, local) = helper::destination(repo, remote, p.source_scope.as_ref(), c)?;
    if Some(&destination) != p.source.as_ref() || local != p.local_source {
        return fail(LfsFailure::Stale);
    }
    let _cache = disk::prepare_store(repo, &entries)?;
    let _destination = if let Some(path) = &local {
        let target = GitClient::from_runner(repo.runner.clone()).open(path, c)?;
        let mut guards = disk::prepare_store(&target, &entries)?;
        // A pre-existing object with the same ID must be valid, never overwritten
        // as a way to repair arbitrary remote files.
        for entry in &entries {
            guards.extend(disk::validate_existing_object(
                &target,
                &p.helper,
                &entry.object,
                c,
            )?);
        }
        guards
    } else {
        Vec::new()
    };
    repo.lfs_recheck(p, c)?;
    let config = helper::push_config(remote, &destination);
    let input = entries
        .iter()
        .map(|e| format!("{}\n", e.object.oid))
        .collect::<String>()
        .into_bytes();
    update(
        progress,
        FetchPhase::UploadingLfsObjects,
        0,
        entries.len() as u64,
    );
    repo.runner.step("lfs-push", "upload_lfs", || {
        c.check()?;
        *outcome = Some(LfsUploadOutcome::Unknown);
        repo.runner
            .lfs_push(
                &p.helper,
                repo.cwd(),
                &args(&["push", "--object-id", "--stdin", remote]),
                input,
                &config,
                progress,
                c,
            )?
            .success("lfs-push")?;
        *outcome = Some(LfsUploadOutcome::ConfirmedByHelper);
        Ok(())
    })?;
    update(progress, FetchPhase::CheckingRepository, 0, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lfs_upload_failures_remain_typed_and_payload_free() {
        for (text, expected) in [
            (
                "batch response: Authorization error: PRIVATE",
                "authentication_rejected",
            ),
            ("PRIVATE: status code: 403", "access_denied"),
            (
                "PRIVATE: Permission denied (publickey).",
                "ssh_authentication_rejected",
            ),
            (
                "PRIVATE: This repository exceeded its data quota",
                "lfs_upload_quota",
            ),
            (
                "PRIVATE: upload rejected: status code: 422",
                "lfs_upload_rejected",
            ),
            ("PRIVATE: connection refused", "connection_failed"),
            ("PRIVATE unknown output", "lfs_upload_failed"),
            (
                "PRIVATE: x509: certificate signed by unknown authority",
                "tls_verification_failed",
            ),
            (
                "PRIVATE: dial tcp: lookup host: no such host",
                "name_resolution_failed",
            ),
            (
                "PRIVATE: net/http: TLS handshake timeout",
                "connection_timed_out",
            ),
        ] {
            for stdout in [false, true] {
                let error = Error::Git {
                    operation: "lfs-push",
                    code: Some(2),
                    stdout: if stdout {
                        text.as_bytes().to_vec()
                    } else {
                        vec![]
                    },
                    stderr: if stdout {
                        vec![]
                    } else {
                        text.as_bytes().to_vec()
                    },
                };
                assert_eq!(error.lfs_push_failure().unwrap().code(), expected);
                assert!(!error.to_string().contains("PRIVATE"));
            }
        }
        assert!(Error::Cancelled.lfs_push_failure().is_none());
        assert!(Error::TimedOut.lfs_push_failure().is_none());
    }
}
