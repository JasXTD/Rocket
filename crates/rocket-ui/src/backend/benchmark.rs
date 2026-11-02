//! Timed calls reuse the actual UI backend and its limits, never shell commands.
use super::*;
use crate::benchmark::{Operation, PASSES, Progress, Sample, update};
use std::time::Instant;

impl Backend {
    pub fn benchmark(
        &self,
        root: &Path,
        progress: &Progress,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let started = Instant::now();
        let result = self.log.request("benchmark", |_| {
            cancel.check()?;
            let repo = self.request("benchmark-prepare", false, |git| git.open(root, cancel))?;
            let watch = RepositoryWatch::new(repo.info(), Arc::new(|| {})).ok();
            let observe = || -> Result<()> {
                let (changed, unavailable) = match &watch {
                    Some(w) => match (w.take_changed(), w.is_current(), w.metadata_generation()) {
                        (Ok(changed), Ok(true), Some(_)) => (changed, false),
                        (Ok(changed), _, _) => (changed, true),
                        _ => (false, true),
                    },
                    None => (false, true),
                };
                update(progress, |p| {
                    p.changed |= changed;
                    p.observation_unavailable |= unavailable;
                })
            };
            observe()?;
            let initial = repo.head(cancel)?;
            let target = match &initial {
                Head::Branch { target, .. } | Head::Detached { target } => Some(target.clone()),
                Head::Unborn { .. } => None,
            };
            for pass in 1..=PASSES {
                for operation in Operation::ALL {
                    cancel.check()?;
                    observe()?;
                    update(progress, |p| {
                        p.active = Some(operation);
                        p.pass = pass;
                        p.elapsed_ms = started.elapsed().as_millis() as u64;
                    })?;
                    let tick = Instant::now();
                    let measured = self.log.request(operation.log_label(), |_| {
                        let result = match operation {
                            Operation::Refresh => {
                                let s = self.snapshot_observed(
                                    root,
                                    HistoryStart::Head,
                                    cancel,
                                    Some(Arc::new(|| {})),
                                    false,
                                )?;
                                if !s.warnings.is_empty() {
                                    return Err(Error::InvalidInput(format!(
                                        "Refresh was incomplete: {}",
                                        s.warnings.join("; ")
                                    )));
                                }
                                Ok(Some((
                                    s.status.as_ref().map_or(0, |s| s.entries.len()),
                                    false,
                                )))
                            }
                            Operation::History => match &target {
                                Some(id) => {
                                    let h = self.history(
                                        &repo,
                                        HistoryStart::Commit(id.clone()),
                                        cancel,
                                    )?;
                                    Ok(Some((h.commits.len(), h.next.is_some())))
                                }
                                None => Ok(None),
                            },
                            Operation::Diff => match &target {
                                Some(id) => {
                                    let p = self.diff(
                                        &repo,
                                        &DiffOptions {
                                            target: DiffTarget::Commit { id: id.clone() },
                                            ..Default::default()
                                        },
                                        cancel,
                                    )?;
                                    Ok(Some((p.files.len(), p.limited)))
                                }
                                None => Ok(None),
                            },
                        };
                        cancel.check()?;
                        result
                    });
                    // Keep completed samples only. Cancellation/failure time is never a success sample.
                    let elapsed = tick.elapsed().as_micros().min(u64::MAX as u128) as u64;
                    let measured = measured?;
                    update(progress, |p| {
                        p.samples.push(Sample {
                            operation,
                            pass,
                            elapsed_us: measured.map(|_| elapsed),
                            items: measured.map_or(0, |v| v.0),
                            limited: measured.is_some_and(|v| v.1),
                        })
                    })?;
                    observe()?;
                }
            }
            // A fresh semantic HEAD check supplements native notifications.
            if repo.head(cancel)? != initial {
                update(progress, |p| p.changed = true)?;
            }
            observe()?;
            cancel.check()
        });
        update(progress, |p| {
            p.elapsed_ms = started.elapsed().as_millis() as u64;
            p.failure = result.as_ref().err().map(|e| match e {
                Error::Cancelled => "cancelled",
                Error::TimedOut => "timeout",
                Error::OutputLimit { .. } => "output_limit",
                Error::WorkerPanicked => "worker_failed",
                _ => "read_failed",
            });
        })?;
        result
    }
}
