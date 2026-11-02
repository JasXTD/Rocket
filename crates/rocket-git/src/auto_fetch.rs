//! Automatic fetch: every configured remote with the checks of a manual fetch,
//! never asking for input. Nothing is pruned, merged or checked out.
use crate::{runner::args, *};

/// What one automatic fetch observed, remote by remote.
#[derive(Debug, Default)]
pub struct QuietFetchReport {
    pub results: Vec<(Vec<u8>, Result<FetchOutput>)>,
    /// Remotes not attempted after cancellation.
    pub skipped: Vec<Vec<u8>>,
}
impl QuietFetchReport {
    /// Whether any remote-tracking branch changed. Porcelain output also lists
    /// branches that were already up to date.
    pub fn changed(&self) -> bool {
        self.results.iter().any(|(_, result)| {
            result.as_ref().is_ok_and(|o| {
                o.updates.as_ref().is_ok_and(|updates| {
                    updates.iter().any(|u| u.kind != FetchUpdateKind::UpToDate)
                })
            })
        })
    }
}

impl Repository {
    /// Fetch every configured remote, one at a time, with the checks of
    /// [`Repository::fetch`]: pinned branch refspecs, `tags` as chosen, no pruning and
    /// no submodules. Unlike a manual fetch it never prompts: no askpass program runs,
    /// Git Credential Manager and SSH are told not to ask, and a remote that needs
    /// input fails instead. The write gate is held per remote, so cancelling frees
    /// it quickly; cancellation stops before the next remote.
    pub fn fetch_all_quietly(&self, tags: FetchTags, cancel: &CancellationToken) -> Result<QuietFetchReport> {
        let mut report = QuietFetchReport::default();
        for remote in self.remotes(cancel)? {
            if cancel.is_cancelled() {
                report.skipped.push(remote.name);
                continue;
            }
            let result = self.fetch_quietly(&remote.name, tags, cancel);
            report.results.push((remote.name, result));
        }
        Ok(report)
    }

    fn fetch_quietly(&self, name: &[u8], tags: FetchTags, cancel: &CancellationToken) -> Result<FetchOutput> {
        let (prefix, remote_argument) = self.validate_fetch_selection(name, cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let (mut fetch, input) = self
            .runner
            .step("fetch", "validate_remote_and_refspecs", || {
                self.prepare_fetch(name, &prefix, remote_argument, cancel)
            })?;
        tags.apply(&mut fetch);
        // Git Credential Manager reads this; the runner removes every askpass route.
        let mut arguments = args(&["-c", "credential.interactive=never"]);
        arguments.extend(fetch);
        self.execute_quiet_fetch(&arguments, input, cancel)
    }
}
