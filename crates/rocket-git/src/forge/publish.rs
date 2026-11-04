//! Reviewed creation + first push. API credentials never become Git credentials.
use super::*;
use crate::{local_files, runner::args};
use std::{fs, io::Write, path::Path, sync::atomic::AtomicU8};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitHubPublishTransport {
    #[default]
    Https,
    Ssh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPublishDraft {
    pub owner: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    pub branch: String,
    pub remote: String,
    pub transport: GitHubPublishTransport,
    pub set_upstream: bool,
}

pub struct GitHubPublishPreview {
    draft: GitHubPublishDraft,
    git_dir: PathBuf,
    config: Vec<u8>,
    effective: Vec<u8>,
    commit: ObjectId,
    destination: String,
    session: u64,
    created: Instant,
    submitted: AtomicBool,
}
impl fmt::Debug for GitHubPublishPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubPublishPreview")
            .finish_non_exhaustive()
    }
}
impl GitHubPublishPreview {
    pub fn draft(&self) -> &GitHubPublishDraft {
        &self.draft
    }
    pub fn commit(&self) -> &ObjectId {
        &self.commit
    }
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/{}",
            self.draft.owner, self.draft.name
        )
    }
    pub fn destination(&self) -> &str {
        &self.destination
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubCreation {
    NotAttempted,
    Created,
    Rejected,
    Unknown,
}
impl GitHubCreation {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Created => "created",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
#[must_use = "inspect creation, configuration and push separately before retrying"]
pub struct GitHubPublishReport {
    pub creation: GitHubCreation,
    pub remote_added: bool,
    pub push: Option<PushReport>,
    pub error: Option<Error>,
    pub url: String,
}

#[derive(Default)]
pub struct GitHubPublishProgress(AtomicU8);
impl GitHubPublishProgress {
    pub fn phase(&self) -> &'static str {
        match self.0.load(Ordering::Acquire) {
            1 => "Creating GitHub repository",
            2 => "Adding local remote",
            3 => "Pushing reviewed branch",
            4 => "Finished",
            _ => "Checking reviewed repository",
        }
    }
    fn set(&self, phase: u8) {
        self.0.store(phase, Ordering::Release);
    }
}

impl ForgeSession {
    /// Synthetic API tests only. Destinations are pre-created bare repositories
    /// named <repository>.git under this explicit operator-selected directory.
    /// Never consume a local path or clone URL supplied by an API response.
    pub fn with_publish_fixture_root(mut self, root: &Path) -> Result<Self> {
        if self.config.fixture_port().is_none() {
            return Err(invalid(
                "Publishing fixtures require an explicit loopback hosting API.",
            ));
        }
        local_files::directory(root)?;
        self.publish_fixture_root =
            Some(fs::canonicalize(root).map_err(|e| Error::io("resolve publish fixture", e))?);
        Ok(self)
    }
    fn publish_destination(&self, draft: &GitHubPublishDraft) -> Result<String> {
        if self.config.fixture_port().is_some() {
            let root=self.publish_fixture_root.as_ref().ok_or_else(||invalid("publish_fixture_required: this loopback session requires an explicit local Git destination."))?;
            let path = root.join(format!("{}.git", draft.name));
            let _pins = local_files::pin_directories(root, &path)?;
            local_files::directory(&path)?;
            let actual =
                fs::canonicalize(&path).map_err(|e| Error::io("resolve publish destination", e))?;
            if actual.parent() != Some(root.as_path()) {
                return Err(invalid(
                    "publish_fixture_escape: destination is outside the fixture root.",
                ));
            }
            // Ordinary drive spelling works with Git for Windows; verbatim paths do not.
            return Ok(actual
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .replace('\\', "/"));
        }
        Ok(match draft.transport {
            GitHubPublishTransport::Https => {
                format!("https://github.com/{}/{}.git", draft.owner, draft.name)
            }
            GitHubPublishTransport::Ssh => {
                format!("git@github.com:{}/{}.git", draft.owner, draft.name)
            }
        })
    }
    fn publish_validate(&self, d: &GitHubPublishDraft) -> Result<()> {
        if self.config.publish_check_repository().is_some_and(|r|
            r != format!("{}/{}", d.owner, d.name) || !d.private) {
            return Err(invalid("hosting_publish_scope: review only the approved private disposable repository."));
        }
        if self.config.provider() != ForgeProvider::GitHub || self.config.account_check_only() {
            return Err(invalid(
                "publish_github_only: connect a GitHub account with repository creation permission.",
            ));
        }
        let owner = |s: &str| {
            !s.is_empty()
                && s.len() <= 39
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        };
        if !owner(&d.owner) || !owner(&self.login) {
            return Err(invalid(
                "publish_owner: enter your GitHub login or an organization name (letters, numbers and hyphens).",
            ));
        }
        if d.name.is_empty()
            || d.name.len() > 100
            || !d.name.as_bytes()[0].is_ascii_alphanumeric()
            || !d
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            || d.name.to_ascii_lowercase().ends_with(".git")
        {
            return Err(invalid(
                "publish_name: use 1–100 letters, numbers, dots, hyphens or underscores, starting with a letter or number and without a .git suffix.",
            ));
        }
        if d.description.len() > 350 || d.description.chars().any(char::is_control) {
            return Err(invalid(
                "publish_description: use a single-line description up to 350 bytes.",
            ));
        }
        validate_branch(&d.branch)?;
        // A simple subsection name allows an exact, appended Git config stanza.
        if d.remote.is_empty()
            || d.remote.len() > 128
            || !d.remote.as_bytes()[0].is_ascii_alphanumeric()
            || !d
                .remote
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(invalid(
                "publish_remote_name: use a new remote name containing letters, numbers, dots, hyphens or underscores.",
            ));
        }
        Ok(())
    }
    fn publish_absent(&self, d: &GitHubPublishDraft, c: &CancellationToken) -> Result<()> {
        if !d.owner.eq_ignore_ascii_case(&self.login) {
            let org = self
                .request(
                    "github-publish-preview",
                    "verify_publish_owner",
                    "GET",
                    &format!("/orgs/{}", d.owner),
                    None,
                    false,
                    c,
                )?
                .0;
            if !org
                .str("login")
                .is_some_and(|s| s.eq_ignore_ascii_case(&d.owner))
            {
                return fail(ForgeFailure::InvalidResponse);
            }
        }
        match self.request(
            "github-publish-preview",
            "check_repository_absent",
            "GET",
            &format!("/repos/{}/{}", d.owner, d.name),
            None,
            false,
            c,
        ) {
            Err(Error::Forge(ForgeFailure::NotFound)) => Ok(()),
            Ok(_) => Err(invalid(
                "publish_repository_exists: that GitHub repository already exists. Choose another name, or use Remotes and Push for an existing repository.",
            )),
            Err(e) => Err(e),
        }
    }
}

impl Repository {
    pub fn preview_github_publish(
        &self,
        s: &ForgeSession,
        d: GitHubPublishDraft,
        c: &CancellationToken,
    ) -> Result<GitHubPublishPreview> {
        self.runner
            .step("github-publish-preview", "review_publish", || {
                s.publish_validate(&d)?;
                let destination = s.publish_destination(&d)?;
                let (config, effective, commit) = self.publish_local(&d, c)?;
                self.publish_scan(&commit, &destination, &d.branch, c)?;
                s.publish_absent(&d, c)?;
                if self.publish_local(&d, c)? != (config.clone(), effective.clone(), commit.clone())
                {
                    return Err(stale());
                }
                Ok(GitHubPublishPreview {
                    draft: d,
                    git_dir: self.info().git_dir.clone(),
                    config,
                    effective,
                    commit,
                    destination,
                    session: s.id,
                    created: Instant::now(),
                    submitted: AtomicBool::new(false),
                })
            })
    }
    fn publish_local(
        &self,
        d: &GitHubPublishDraft,
        c: &CancellationToken,
    ) -> Result<(Vec<u8>, Vec<u8>, ObjectId)> {
        self.validate_managed_remote_name(d.remote.as_bytes(), c)?;
        if !self
            .remote_values(d.remote.as_bytes(), false, c)?
            .is_empty()
        {
            return Err(invalid(
                "publish_remote_exists: this remote name is already configured. Existing remotes are never overwritten; choose another name.",
            ));
        }
        let reference = format!("refs/heads/{}", d.branch);
        let branch=self.branches(c)?.into_iter().find(|b|b.reference==reference.as_bytes()&&b.symbolic_target.is_none()).ok_or_else(||invalid("publish_branch: select a local branch containing a commit; create your first commit before publishing."))?;
        let config = local_files::read_config(&self.info().common_dir.join("config"))?;
        if self
            .info()
            .common_dir
            .join("config.lock")
            .try_exists()
            .map_err(|e| Error::io("inspect publish config lock", e))?
        {
            return Err(invalid(
                "publish_config_locked: another writer owns config.lock. Finish that operation and review again; the existing lock is left in place.",
            ));
        }
        let effective = self
            .runner
            .run(
                self.cwd(),
                &args(&["config", "--null", "--list"]),
                vec![],
                c,
            )?
            .success("publish config")?;
        for line in effective.split(|b| *b == 0) {
            let key = line.split(|b| *b == b'\n').next().unwrap_or_default();
            if key.starts_with(b"url.")
                && (key.ends_with(b".insteadof") || key.ends_with(b".pushinsteadof"))
            {
                return Err(invalid(
                    "publish_url_rewrite: Git URL rewriting is configured. Review it before publishing; Rocket will not send this first push to a rewritten destination.",
                ));
            }
            if d.set_upstream && key == format!("branch.{}.remote", d.branch).as_bytes() {
                return Err(invalid(
                    "publish_upstream_exists: this branch already has tracking settings. Turn off Set upstream to preserve them.",
                ));
            }
        }
        Ok((config, effective, branch.target))
    }
    fn publish_scan(
        &self,
        commit: &ObjectId,
        url: &str,
        branch: &str,
        c: &CancellationToken,
    ) -> Result<()> {
        if self.runner.content_guard_enabled() {
            self.runner
                .step("github-publish-preview", "scan_publish_history", || {
                    crate::secret_guard::check(
                        &self.runner,
                        self.cwd(),
                        &args(&["push", "--", url, &format!("{commit}:refs/heads/{branch}")]),
                        &[],
                        c,
                    )
                })?;
        }
        Ok(())
    }
    fn publish_revalidate(
        &self,
        s: &ForgeSession,
        p: &GitHubPublishPreview,
        c: &CancellationToken,
    ) -> Result<()> {
        if p.git_dir != self.info().git_dir
            || p.session != s.id
            || p.created.elapsed() > Duration::from_secs(120)
            || p.destination != s.publish_destination(&p.draft)?
        {
            return Err(stale());
        }
        let (config, effective, commit) = self.publish_local(&p.draft, c)?;
        if config != p.config || effective != p.effective || commit != p.commit {
            return Err(stale());
        }
        Ok(())
    }

    /// A single write permit spans API creation, config attachment and first push.
    /// No checkout/index write, implicit commit, overwrite, force replacement or
    /// automatic retry/deletion occurs. Partial effects remain explicit in report.
    pub fn publish_github(
        &self,
        s: &ForgeSession,
        p: &GitHubPublishPreview,
        progress: &GitHubPublishProgress,
        c: &CancellationToken,
    ) -> Result<GitHubPublishReport> {
        let _permit = self.acquire_write(c)?;
        self.runner
            .step("github-publish", "revalidate_publish", || {
                if p.submitted.load(Ordering::Acquire) {
                    return Err(stale());
                }
                self.publish_revalidate(s, p, c)?;
                self.publish_scan(&p.commit, &p.destination, &p.draft.branch, c)?;
                s.publish_absent(&p.draft, c)?;
                self.publish_revalidate(s, p, c)
            })?;
        c.check()?;
        if p.submitted.swap(true, Ordering::AcqRel) {
            return Err(stale());
        }
        let mut report = GitHubPublishReport {
            creation: GitHubCreation::NotAttempted,
            remote_added: false,
            push: None,
            error: None,
            url: p.url(),
        };
        progress.set(1);
        let d = &p.draft;
        let endpoint = if d.owner.eq_ignore_ascii_case(&s.login) {
            "/user/repos".into()
        } else {
            format!("/orgs/{}/repos", d.owner)
        };
        let body = format!(
            "{{\"name\":{},\"description\":{},\"private\":{},\"auto_init\":false}}",
            quote(&d.name),
            quote(&d.description),
            d.private
        );
        let created = s
            .request(
                "github-publish",
                "create_github_repository",
                "POST",
                &endpoint,
                Some(&body),
                true,
                c,
            )
            .and_then(|(v, _)| {
                if v.u64("id").is_none_or(|id| id == 0)
                    || v.bool("private") != Some(d.private)
                    || !v
                        .str("full_name")
                        .is_some_and(|n| n.eq_ignore_ascii_case(&format!("{}/{}", d.owner, d.name)))
                {
                    return fail(ForgeFailure::OutcomeUnknown);
                }
                Ok(())
            });
        if let Err(e) = created {
            report.creation = match &e {
                Error::Cancelled => GitHubCreation::NotAttempted,
                Error::Forge(ForgeFailure::OutcomeUnknown) => GitHubCreation::Unknown,
                _ => GitHubCreation::Rejected,
            };
            report.error = Some(publish_error(e));
            if let Some(log) = &self.runner.log {
                log.github_publish_outcome(&report);
            }
            progress.set(4);
            return Ok(report);
        }
        report.creation = GitHubCreation::Created;
        let result = (|| {
            progress.set(2);
            self.runner
                .step("github-publish", "attach_publish_remote", || {
                    self.publish_revalidate(s, p, c)?;
                    self.publish_attach(p, c)
                })?;
            report.remote_added = true;
            c.check()?;
            progress.set(3);
            report.push = Some(self.push_locked(
                d.remote.as_bytes(),
                d.branch.as_bytes(),
                d.branch.as_bytes(),
                PushOptions {
                    set_upstream: d.set_upstream,
                },
                Some((&p.commit, p.destination.as_bytes())),
                c,
            )?);
            Ok(())
        })();
        if let Err(e) = result {
            report.error = Some(e);
        }
        if let Some(log) = &self.runner.log {
            log.github_publish_outcome(&report);
        }
        progress.set(4);
        Ok(report)
    }
    fn publish_attach(&self, p: &GitHubPublishPreview, c: &CancellationToken) -> Result<()> {
        let root = &self.info().common_dir;
        let _pins = local_files::pin_directories(root, root)?;
        let config = root.join("config");
        let lock = root.join("config.lock");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| Error::io("lock publish remote configuration", e))?;
        let result = (|| {
            if local_files::read_config(&config)? != p.config {
                return Err(stale());
            }
            file.set_permissions(
                fs::metadata(&config)
                    .map_err(|e| Error::io("inspect publish configuration", e))?
                    .permissions(),
            )
            .map_err(|e| Error::io("preserve config permissions", e))?;
            let mut contents = p.config.clone();
            contents.extend_from_slice(
                format!(
                    "\n[remote {}]\n\turl = {}\n\tfetch = {}\n",
                    quote(&p.draft.remote),
                    quote(&p.destination),
                    quote(&format!("+refs/heads/*:refs/remotes/{}/*", p.draft.remote))
                )
                .as_bytes(),
            );
            file.write_all(&contents)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io("write publish remote configuration", e))?;
            c.check()?;
            if local_files::read_config(&config)? != p.config {
                return Err(stale());
            }
            Ok(())
        })();
        drop(file);
        if let Err(e) = result {
            let _ = fs::remove_file(&lock);
            return Err(e);
        }
        let moved = local_files::move_path(&lock, &config, true);
        if moved.is_err() {
            let _ = fs::remove_file(&lock);
        }
        moved
    }
}
fn stale() -> Error {
    invalid(
        "publish_stale: the branch, account or Git configuration changed, or the review expired. Review again; a repository already created on GitHub is retained.",
    )
}
fn publish_error(e: Error) -> Error {
    match e {
        Error::Forge(ForgeFailure::Rejected) => invalid(
            "publish_rejected: GitHub refused repository creation. Check the name, visibility, organization rules and account permissions. No local remote was added and no branch was pushed.",
        ),
        Error::Forge(ForgeFailure::OutcomeUnknown) => invalid(
            "publish_creation_unknown: GitHub may have created the repository before the response was lost. Inspect the reviewed GitHub URL before trying again. No local remote was added and no branch was pushed; Rocket will not retry or delete it automatically.",
        ),
        other => other,
    }
}
