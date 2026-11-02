//! Test connection: can Git reach a remote and list its branches without asking
//! for anything? `git ls-remote` runs with the automatic fetch's no-prompt
//! settings and its own time limit. Nothing is downloaded or written.
use crate::{
    clone::source_argument,
    model::bytes_to_os,
    remote_management::validate_url,
    runner::{Runner, args},
    *,
};
use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};

/// How long Test connection waits for an answer.
pub const REMOTE_CHECK_LIMIT: Duration = Duration::from_secs(20);

/// What a successful connection test saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCheck {
    pub branches: usize,
    /// The branch the remote's HEAD names (`main`), when it says so.
    pub default_branch: Option<Vec<u8>>,
    pub elapsed: Duration,
}

impl Repository {
    /// Test a URL or path typed for one of this repository's remotes. The
    /// repository's own configuration applies (URL rewrites, proxies, SSH
    /// command), as it would for a fetch.
    pub fn check_remote(&self, url: &[u8], cancel: &CancellationToken) -> Result<RemoteCheck> {
        let url = self
            .runner
            .step("remote-check", "validate_remote_url", || {
                validate_url(url)?;
                bytes_to_os(url)
            })?;
        check(&self.runner, self.cwd(), url, None, cancel)
    }
}

impl GitClient {
    /// Test a clone source with the clone's rules: an https or ssh URL or an
    /// existing local folder, and the clone's protocol allowlist. It runs in
    /// `folder`, which must not be inside a repository whose settings matter.
    pub fn check_clone_source(
        &self,
        source: &CloneSource,
        folder: &Path,
        cancel: &CancellationToken,
    ) -> Result<RemoteCheck> {
        let url = self
            .runner
            .step("remote-check", "validate_clone_source", || {
                source_argument(source)
            })?;
        let local_only = matches!(source, CloneSource::Local(_));
        check(&self.runner, folder, url, Some(local_only), cancel)
    }
}

fn check(
    runner: &Runner,
    cwd: &Path,
    url: OsString,
    clone_local_only: Option<bool>,
    cancel: &CancellationToken,
) -> Result<RemoteCheck> {
    let mut runner = runner.clone();
    runner.limits.timeout = REMOTE_CHECK_LIMIT;
    let started = Instant::now();
    // Git Credential Manager reads this; the runner removes every askpass route.
    let mut arguments = args(&[
        "-c",
        "credential.interactive=never",
        "ls-remote",
        "--symref",
        "--",
    ]);
    arguments.extend([url, "HEAD".into(), "refs/heads/*".into()]);
    let output = runner.observe("remote_check", "remote-check", || {
        runner
            .remote_check(cwd, &arguments, clone_local_only, cancel)?
            .success("remote-check")
    })?;
    let mut check = parse(&output)?;
    check.elapsed = started.elapsed();
    Ok(check)
}

/// `ref: <target>\t<name>` and `<id>\t<name>` lines. The patterns also match
/// names ending in `/HEAD`; only `HEAD` itself names the default branch.
fn parse(output: &[u8]) -> Result<RemoteCheck> {
    let mut check = RemoteCheck {
        branches: 0,
        default_branch: None,
        elapsed: Duration::ZERO,
    };
    for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let (value, name) = line
            .iter()
            .position(|b| *b == b'\t')
            .map(|tab| (&line[..tab], &line[tab + 1..]))
            .ok_or_else(|| Error::parse("remote-check", "malformed ls-remote line"))?;
        if let Some(target) = value.strip_prefix(b"ref: ") {
            if name == b"HEAD" {
                check.default_branch = Some(
                    target
                        .strip_prefix(b"refs/heads/")
                        .unwrap_or(target)
                        .to_vec(),
                );
            }
        } else if name.starts_with(b"refs/heads/") {
            check.branches += 1;
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertisement_counts_branches_and_names_the_default() {
        let check = parse(
            b"ref: refs/heads/main\tHEAD\n1111111111111111111111111111111111111111\tHEAD\n\
              1111111111111111111111111111111111111111\trefs/heads/main\n\
              2222222222222222222222222222222222222222\trefs/heads/feature/x\n\
              ref: refs/remotes/origin/main\trefs/remotes/origin/HEAD\n",
        )
        .unwrap();
        assert_eq!(check.branches, 2);
        assert_eq!(check.default_branch.as_deref(), Some(&b"main"[..]));
        let empty = parse(b"").unwrap();
        assert_eq!((empty.branches, empty.default_branch), (0, None));
        assert!(parse(b"no tab here\n").is_err());
    }
}
