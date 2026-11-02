//! One read-only, cancellable policy check at a time. UI state never crosses
//! threads; the caller pins its input binding and rechecks it before delivery.
use crate::policy::{GITHUB_TEST_SOURCES, Policy};
use rocket_git::{
    CancellationToken, DiagnosticLog, Error, InputValidationStep as Step, Repository,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ForcePush,
    AtomicPush,
    RemoteTag,
    Pull,
}
impl Kind {
    pub fn operation(self) -> &'static str {
        match self {
            Self::ForcePush => "force-push-policy",
            Self::AtomicPush => "atomic-push-policy",
            Self::RemoteTag => "remote-tag-policy",
            Self::Pull => "pull-policy",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Spec {
    pub kind: Kind,
    pub path: PathBuf,
    pub remote: Vec<u8>,
    pub branch: String,
}
impl Spec {
    fn url(&self, policy: &Policy, value: &[u8]) -> rocket_git::Result<()> {
        let result = match self.kind {
            Kind::ForcePush => policy.force_push_url(&self.path, value),
            Kind::AtomicPush => policy.atomic_push_url(&self.path, value),
            Kind::RemoteTag => policy.remote_url(value),
            Kind::Pull
                if std::str::from_utf8(value).is_ok_and(|s| GITHUB_TEST_SOURCES.contains(&s)) =>
            {
                Ok(())
            }
            Kind::Pull => policy.remote_url(value),
        };
        result.map_err(Error::InvalidInput)
    }
    fn check(
        &self,
        policy: &Policy,
        repo: &Repository,
        log: &DiagnosticLog,
        token: &CancellationToken,
        stage: &AtomicU8,
    ) -> rocket_git::Result<()> {
        let step = |s: Step| {
            stage.store(s as u8, Ordering::Release);
        };
        step(Step::Fixture);
        log.input_validation_step(Step::Fixture, || {
            token.check()?;
            let (_, branch) = policy.repository(&self.path).map_err(Error::InvalidInput)?;
            if branch != self.branch {
                return Err(Error::InvalidInput(
                    "Fixture ownership changed during validation".into(),
                ));
            }
            Ok(())
        })?;
        step(Step::Configuration);
        let before =
            log.input_validation_step(Step::Configuration, || repo.configuration_snapshot(token))?;
        step(Step::Remotes);
        log.input_validation_step(Step::Remotes, || {
            let remotes = repo.remotes(token)?;
            let remote = remotes
                .iter()
                .find(|r| r.name == self.remote)
                .ok_or_else(|| Error::InvalidInput("Remote no longer exists".into()))?;
            if self.kind == Kind::Pull
                && (remotes.len() != 1
                    || remote.name != b"origin"
                    || remote.fetch_refspecs
                        != vec![
                            format!(
                                "+refs/heads/{}:refs/remotes/origin/{}",
                                self.branch, self.branch
                            )
                            .into_bytes(),
                        ])
            {
                return Err(Error::InvalidInput(
                    "Origin must map only the owned fixture branch".into(),
                ));
            }
            for url in remote.urls.iter().chain(&remote.push_urls) {
                self.url(policy, url)?;
            }
            Ok(())
        })?;
        if self.kind != Kind::Pull {
            step(Step::Destination);
            log.input_validation_step(Step::Destination, || {
                let destination = match self.kind {
                    Kind::ForcePush | Kind::AtomicPush => repo.force_push_destination(&self.remote, token),
                    _ => repo.remote_tag_destination(&self.remote, token),
                };
                match destination {
                    Ok(url) => self.url(policy, &url),
                    // The backend renders these unsupported-selection refusals
                    // before transport, as it did in the synchronous guard.
                    Err(Error::ForcePush(rocket_git::ForcePushFailure::DestinationUnsupported))
                    | Err(Error::Tag(rocket_git::TagFailure::RemoteUnsupported)) => Ok(()),
                    Err(error) => Err(error),
                }
            })?;
        }
        step(Step::Recheck);
        log.input_validation_step(Step::Recheck, || {
            token.check()?;
            if repo.configuration_snapshot(token)? != before {
                return Err(Error::InvalidInput(
                    "Git configuration changed during validation; review again".into(),
                ));
            }
            let (_, branch) = policy.repository(&self.path).map_err(Error::InvalidInput)?;
            if branch != self.branch {
                return Err(Error::InvalidInput(
                    "Fixture ownership changed during validation".into(),
                ));
            }
            Ok(())
        })
    }
}

pub struct Job {
    pub spec: Spec,
    pub started: Instant,
    token: CancellationToken,
    stage: Arc<AtomicU8>,
    result: Receiver<(Result<(), String>, Instant)>,
    worker: Option<JoinHandle<()>>,
}
impl Job {
    pub fn start(
        spec: Spec,
        policy: Policy,
        repo: Repository,
        log: DiagnosticLog,
        deadline: Instant,
    ) -> Result<Self, String> {
        let token = CancellationToken::new();
        let stage = Arc::new(AtomicU8::new(Step::Fixture as u8));
        let (sender, result) = mpsc::sync_channel(1);
        let (work_spec, work_token, work_stage) = (spec.clone(), token.clone(), stage.clone());
        let worker = thread::Builder::new().name("rocket-input-validation".into()).spawn(move || {
            let outcome = log.request(work_spec.kind.operation(), |log| {
                let result = work_spec.check(&policy, &repo.with_log(log.clone()), log, &work_token, &work_stage);
                if Instant::now() >= deadline { Err(Error::TimedOut) } else { result }
            }).map_err(|error| match error {
                Error::InvalidInput(message) => format!("{message}; no input was delivered"),
                Error::Cancelled => "Input validation cancelled; no input was delivered".into(),
                Error::TimedOut => "Git validation timed out; no input was delivered. Inspect the helper and retry explicitly".into(),
                _ => "Cannot validate the current fixture destination; no input was delivered. Inspect validation diagnostics".into(),
            });
            let _ = sender.send((outcome, Instant::now()));
        }).map_err(|_| "Cannot start input validation; no input was delivered")?;
        Ok(Self {
            spec,
            started: Instant::now(),
            token,
            stage,
            result,
            worker: Some(worker),
        })
    }
    pub fn cancel(&self) {
        self.token.cancel();
    }
    pub fn cancelling(&self) -> bool {
        self.token.is_cancelled()
    }
    pub fn step(&self) -> &'static str {
        match self.stage.load(Ordering::Acquire) {
            0 => Step::Configuration,
            1 => Step::Remotes,
            2 => Step::Destination,
            3 => Step::Recheck,
            _ => Step::Fixture,
        }
        .code()
    }
    /// Never join a running worker on the UI thread. Retain the slot through
    /// cleanup, even after cancellation, so repeated requests cannot leak workers.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        let worker = self.worker.take().unwrap();
        if worker.join().is_err() {
            return Some(Err(
                "Input validation worker failed; no input was delivered".into(),
            ));
        }
        Some(match self.result.try_recv() {
            Ok((result, finished)) if finished.elapsed() <= Duration::from_secs(1) => result,
            Ok(_) => Err(
                "Input validation became stale before delivery; read state and review again".into(),
            ),
            Err(_) => Err("Input validation ended without a result; no input was delivered".into()),
        })
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.token.cancel();
    }
}
