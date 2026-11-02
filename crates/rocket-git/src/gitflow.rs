//! Native git-flow with plain Git commands: git-flow's own `gitflow.*` settings, and
//! reviewed start/finish of feature, release, hotfix and support branches. Each write
//! reuses Rocket's branch, merge, tag and config transactions under one write gate.
use crate::{local_files, runner::args, *};
use std::{ffi::OsString, path::PathBuf};
mod finish;
mod start;
pub use finish::{
    GitFlowFinishOptions, GitFlowFinishPreview, GitFlowFinishReport, GitFlowKeep, GitFlowNote,
    GitFlowStep, GitFlowStepReport,
};
pub use start::{GitFlowStartPreview, GitFlowStartReport};

/// The `gitflow.*` keys git-flow (nvie and AVH editions) reads, in [`GitFlowConfig`] order.
pub const GITFLOW_KEYS: [&str; 7] = [
    "gitflow.branch.master",
    "gitflow.branch.develop",
    "gitflow.prefix.feature",
    "gitflow.prefix.release",
    "gitflow.prefix.hotfix",
    "gitflow.prefix.support",
    "gitflow.prefix.versiontag",
];
const KEY_PATTERN: &str =
    "^gitflow\\.(branch\\.(master|develop)|prefix\\.(feature|release|hotfix|support|versiontag))$";
/// Longest branch name, prefix or version accepted here.
pub const GITFLOW_NAME_LIMIT: usize = 200;
/// Longest annotated tag message for a finished release or hotfix.
pub const GITFLOW_MESSAGE_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFlowFailure {
    NotInitialized,
    InvalidConfig,
    ProductionMissing,
    DevelopMissing,
    InvalidName,
    BranchExists,
    BranchMissing,
    BaseMissing,
    ReleaseInProgress,
    HotfixInProgress,
    TagExists,
    TagMessage,
    DirtyWorktree,
    ActiveOperation,
    BehindRemote,
    CheckedOutElsewhere,
    NotFinishable,
    NothingToDo,
    StalePreview,
    Conflicts,
    StepFailed,
}
impl GitFlowFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotInitialized => "gitflow_not_initialized",
            Self::InvalidConfig => "gitflow_invalid_config",
            Self::ProductionMissing => "gitflow_production_missing",
            Self::DevelopMissing => "gitflow_develop_missing",
            Self::InvalidName => "gitflow_invalid_name",
            Self::BranchExists => "gitflow_branch_exists",
            Self::BranchMissing => "gitflow_branch_missing",
            Self::BaseMissing => "gitflow_base_missing",
            Self::ReleaseInProgress => "gitflow_release_in_progress",
            Self::HotfixInProgress => "gitflow_hotfix_in_progress",
            Self::TagExists => "gitflow_tag_exists",
            Self::TagMessage => "gitflow_tag_message",
            Self::DirtyWorktree => "gitflow_dirty_worktree",
            Self::ActiveOperation => "gitflow_active_operation",
            Self::BehindRemote => "gitflow_behind_remote",
            Self::CheckedOutElsewhere => "gitflow_checked_out_elsewhere",
            Self::NotFinishable => "gitflow_not_finishable",
            Self::NothingToDo => "gitflow_nothing_to_do",
            Self::StalePreview => "gitflow_stale_preview",
            Self::Conflicts => "gitflow_conflicts",
            Self::StepFailed => "gitflow_step_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::NotInitialized => {
                "Git Flow is not set up for this repository. Use Repository > Git Flow > Initialize first."
            }
            Self::InvalidConfig => {
                "Use two different, valid branch names and distinct, non-empty prefixes. Neither branch may start with a prefix, and no prefix may start with another."
            }
            Self::ProductionMissing => {
                "The production branch does not exist locally. Choose an existing branch, or create or fetch it first."
            }
            Self::DevelopMissing => {
                "The development branch named in gitflow.branch.develop does not exist locally. Initialize Git Flow again to create it."
            }
            Self::InvalidName => {
                "Enter a name that makes a valid branch name (and, for releases and hotfixes, a valid tag name)."
            }
            Self::BranchExists => {
                "A branch with this name exists already. Choose another name, or finish the existing branch."
            }
            Self::BranchMissing => {
                "This git-flow branch no longer exists locally. Refresh and choose another branch."
            }
            Self::BaseMissing => {
                "The base branch or commit could not be resolved locally. Check the name, fetch explicitly if needed, then review again."
            }
            Self::ReleaseInProgress => {
                "A release branch exists already. git-flow keeps one release at a time; finish or delete it first."
            }
            Self::HotfixInProgress => {
                "A hotfix branch exists already. git-flow keeps one hotfix at a time; finish or delete it first."
            }
            Self::TagExists => {
                "A tag with this version exists already and does not contain this branch. Choose another version, or inspect the tag in Tags."
            }
            Self::TagMessage => {
                "Enter a message for the annotated version tag (at most 64 KiB, without NUL characters)."
            }
            Self::DirtyWorktree => {
                "Commit or stash tracked changes, including staged ones, then review again. Untracked files may stay."
            }
            Self::ActiveOperation => {
                "Finish or abort the merge, rebase, cherry-pick, revert or bisect in progress, then review again."
            }
            Self::BehindRemote => {
                "A branch this uses is behind or has diverged from its origin counterpart. Pull it first (Rocket never fetches here), then review again."
            }
            Self::CheckedOutElsewhere => {
                "A branch this would switch to is checked out in another worktree. Switch that worktree to another branch, or finish there."
            }
            Self::NotFinishable => {
                "Support branches are long-lived; git-flow has no finish step for them."
            }
            Self::NothingToDo => {
                "Every step of this finish is done already and the branch is kept. Nothing would change."
            }
            Self::StalePreview => {
                "The repository changed since this review. Nothing was written. Review again."
            }
            Self::Conflicts => {
                "A merge stopped with conflicts. Resolve them in Changes and Continue merge, then choose Finish again: the completed steps are skipped."
            }
            Self::StepFailed => {
                "A step did not complete. Completed steps stay done and later steps did not run. Inspect the activity details and the repository, then Finish again to continue."
            }
        }
    }
}
fn refused<T>(failure: GitFlowFailure) -> Result<T> {
    Err(Error::GitFlow(failure))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFlowKind {
    Feature,
    Release,
    Hotfix,
    Support,
}
impl GitFlowKind {
    pub const ALL: [Self; 4] = [Self::Feature, Self::Release, Self::Hotfix, Self::Support];
    pub fn label(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Release => "release",
            Self::Hotfix => "hotfix",
            Self::Support => "support",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Feature => "Feature",
            Self::Release => "Release",
            Self::Hotfix => "Hotfix",
            Self::Support => "Support",
        }
    }
    /// Releases, hotfixes and support branches are named by a version; finishing a
    /// release or hotfix tags it.
    pub fn versioned(self) -> bool {
        self != Self::Feature
    }
    pub fn tagged(self) -> bool {
        matches!(self, Self::Release | Self::Hotfix)
    }
    /// Support branches are long-lived: git-flow has no finish for them.
    pub fn finishes(self) -> bool {
        self != Self::Support
    }
}

/// git-flow's settings, as stored under `gitflow.*` in Git config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowConfig {
    /// The production branch (git-flow's `master`), for example `main`.
    pub master: String,
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    pub support: String,
    /// Put before a version to name its tag; often empty or `v`.
    pub version_tag: String,
}
impl GitFlowConfig {
    /// git-flow's defaults with this production branch.
    pub fn defaults(master: &str) -> Self {
        Self {
            master: master.into(),
            develop: "develop".into(),
            feature: "feature/".into(),
            release: "release/".into(),
            hotfix: "hotfix/".into(),
            support: "support/".into(),
            version_tag: String::new(),
        }
    }
    pub fn prefix(&self, kind: GitFlowKind) -> &str {
        match kind {
            GitFlowKind::Feature => &self.feature,
            GitFlowKind::Release => &self.release,
            GitFlowKind::Hotfix => &self.hotfix,
            GitFlowKind::Support => &self.support,
        }
    }
    fn values(&self) -> [&str; 7] {
        [
            &self.master,
            &self.develop,
            &self.feature,
            &self.release,
            &self.hotfix,
            &self.support,
            &self.version_tag,
        ]
    }
    fn from_values(values: &[Option<String>; 7]) -> Option<Self> {
        let value = |i: usize| values[i].clone();
        Some(Self {
            master: value(0)?,
            develop: value(1)?,
            feature: value(2)?,
            release: value(3)?,
            hotfix: value(4)?,
            support: value(5)?,
            version_tag: value(6)?,
        })
    }
    /// The local branch a git-flow name makes, for example `feature/login`.
    pub fn branch_name(&self, kind: GitFlowKind, name: &str) -> String {
        format!("{}{name}", self.prefix(kind))
    }
    /// The tag a release or hotfix version gets when finished.
    pub fn tag_name(&self, version: &str) -> String {
        format!("{}{version}", self.version_tag)
    }
    /// The kind and name of a local branch under one of the prefixes.
    pub fn classify<'a>(&self, branch: &'a str) -> Option<(GitFlowKind, &'a str)> {
        GitFlowKind::ALL.into_iter().find_map(|kind| {
            branch
                .strip_prefix(self.prefix(kind))
                .filter(|name| !name.is_empty())
                .map(|name| (kind, name))
        })
    }
    /// Text checks only; reviews also ask Git whether the names make valid refs.
    pub fn validate(&self) -> Result<()> {
        let prefixes = [&self.feature, &self.release, &self.hotfix, &self.support];
        let valid = self.values().iter().all(|v| {
            v.len() <= GITFLOW_NAME_LIMIT && !v.chars().any(|c| c.is_control() || c.is_whitespace())
        }) && !self.master.is_empty()
            && !self.develop.is_empty()
            && self.master != self.develop
            && prefixes.iter().all(|p| !p.is_empty())
            && prefixes.iter().enumerate().all(|(i, a)| {
                prefixes
                    .iter()
                    .enumerate()
                    .all(|(j, b)| i == j || !a.starts_with(b.as_str()))
            })
            && prefixes.iter().all(|p| {
                !self.master.starts_with(p.as_str()) && !self.develop.starts_with(p.as_str())
            });
        if valid {
            Ok(())
        } else {
            refused(GitFlowFailure::InvalidConfig)
        }
    }
}

/// One local branch under a git-flow prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowBranch {
    pub kind: GitFlowKind,
    /// The name after the prefix, for example `login` for `feature/login`.
    pub name: String,
    /// The local branch name.
    pub branch: String,
    pub target: ObjectId,
    pub is_head: bool,
}

/// git-flow's effective settings and the local branches they classify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowState {
    /// The effective settings, when every key git-flow needs is set and valid.
    pub config: Option<GitFlowConfig>,
    /// The effective `gitflow.*` values, in [`GITFLOW_KEYS`] order.
    pub values: [Option<String>; 7],
    /// Local branches under the configured prefixes.
    pub branches: Vec<GitFlowBranch>,
    /// Every local branch name, in ref order.
    pub local: Vec<String>,
    /// The checked-out branch, if any.
    pub head: Option<String>,
}
impl GitFlowState {
    /// Some settings exist, but not a complete, valid configuration.
    pub fn incomplete(&self) -> bool {
        self.config.is_none() && self.values.iter().any(Option::is_some)
    }
    pub fn current(&self) -> Option<&GitFlowBranch> {
        self.branches.iter().find(|b| b.is_head)
    }
    pub fn production_exists(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|c| self.local.contains(&c.master))
    }
    pub fn develop_exists(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|c| self.local.contains(&c.develop))
    }
    /// Values for Initialize: the effective ones where set, else git-flow's defaults,
    /// with `main` or `master` (or the current branch) as the production branch.
    pub fn suggested(&self) -> GitFlowConfig {
        let master = ["main", "master"]
            .into_iter()
            .find(|name| self.local.iter().any(|b| b == name))
            .map(str::to_owned)
            .or_else(|| self.head.clone())
            .unwrap_or_else(|| "main".into());
        let defaults = GitFlowConfig::defaults(&master);
        let mut values = defaults.values().map(str::to_owned);
        for (value, configured) in values.iter_mut().zip(&self.values) {
            if let Some(configured) = configured {
                value.clone_from(configured);
            }
        }
        let [
            master,
            develop,
            feature,
            release,
            hotfix,
            support,
            version_tag,
        ] = values;
        GitFlowConfig {
            master,
            develop,
            feature,
            release,
            hotfix,
            support,
            version_tag,
        }
    }
}

/// How Initialize gets the development branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFlowDevelop {
    /// It exists at this commit.
    Exists(ObjectId),
    /// Created at `target` from `source` (`origin/<develop>`, else the production
    /// branch). `track` also sets `origin/<develop>` as its upstream, as git-flow does.
    Create {
        source: String,
        target: ObjectId,
        track: bool,
    },
}

/// A reviewed Initialize: settings written to the repository's own config, and the
/// development branch created when missing. Nothing is switched or pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowInitPreview {
    repository: PathBuf,
    config: GitFlowConfig,
    production: ObjectId,
    develop: GitFlowDevelop,
    changes: Vec<(&'static str, String)>,
    // The repository's config file as observed; the write refuses if it changed.
    observed: Vec<u8>,
}
impl GitFlowInitPreview {
    pub fn config(&self) -> &GitFlowConfig {
        &self.config
    }
    pub fn production(&self) -> &ObjectId {
        &self.production
    }
    pub fn develop(&self) -> &GitFlowDevelop {
        &self.develop
    }
    /// Repository-local settings this writes, as (key, value).
    pub fn changes(&self) -> &[(&'static str, String)] {
        &self.changes
    }
    /// Already set up exactly like this: nothing would be written.
    pub fn unchanged(&self) -> bool {
        self.changes.is_empty() && matches!(self.develop, GitFlowDevelop::Exists(_))
    }
}

/// Each step is attempted only after the previous one succeeded.
#[derive(Debug)]
#[must_use = "inspect the branch and settings results and the refreshed state"]
pub struct GitFlowInitReport {
    /// Creating the development branch, when it was missing.
    pub branch: Option<BranchWriteReport>,
    /// Writing the settings; `None` when nothing needed writing or an earlier step failed.
    pub config: Option<Result<WriteOutput>>,
    pub state: Result<GitFlowState>,
}

impl Repository {
    /// The effective `gitflow.*` settings, as git-flow reads them, and the local branches.
    pub fn gitflow_state(&self, cancel: &CancellationToken) -> Result<GitFlowState> {
        self.runner.step("gitflow-read", "read_gitflow", || {
            let values = self.gitflow_values(false, cancel)?;
            let branches = self.branches(cancel)?;
            let config = GitFlowConfig::from_values(&values).filter(|c| c.validate().is_ok());
            let mut state = GitFlowState {
                config,
                values,
                branches: Vec::new(),
                local: Vec::new(),
                head: None,
            };
            for branch in branches.iter().filter(|b| b.kind == BranchKind::Local) {
                let Some(name) = branch
                    .reference
                    .strip_prefix(b"refs/heads/")
                    .and_then(|n| std::str::from_utf8(n).ok())
                else {
                    continue;
                };
                if branch.is_head {
                    state.head = Some(name.into());
                }
                state.local.push(name.into());
                if let Some((kind, short)) = state.config.as_ref().and_then(|c| c.classify(name)) {
                    state.branches.push(GitFlowBranch {
                        kind,
                        name: short.into(),
                        branch: name.into(),
                        target: branch.target.clone(),
                        is_head: branch.is_head,
                    });
                }
            }
            Ok(state)
        })
    }

    /// Review Initialize. No checkout, ref or configuration writes.
    pub fn preview_gitflow_init(
        &self,
        config: &GitFlowConfig,
        cancel: &CancellationToken,
    ) -> Result<GitFlowInitPreview> {
        self.runner
            .step("gitflow-init-preview", "review_gitflow_init", || {
                config.validate()?;
                self.check_gitflow_names(config, cancel)?;
                let path = self.info().common_dir.join("config");
                let observed = local_files::read_config(&path)?;
                let local = self.gitflow_values(true, cancel)?;
                let branches = self.branches(cancel)?;
                let production = local_branch(&branches, &config.master)
                    .ok_or(Error::GitFlow(GitFlowFailure::ProductionMissing))?
                    .target
                    .clone();
                let develop = match local_branch(&branches, &config.develop) {
                    Some(branch) => GitFlowDevelop::Exists(branch.target.clone()),
                    None => match origin_branch(&branches, &config.develop) {
                        Some(remote) => GitFlowDevelop::Create {
                            source: format!("origin/{}", config.develop),
                            target: remote.target.clone(),
                            track: true,
                        },
                        None => GitFlowDevelop::Create {
                            source: config.master.clone(),
                            target: production.clone(),
                            track: false,
                        },
                    },
                };
                let changes = GITFLOW_KEYS
                    .iter()
                    .zip(config.values())
                    .zip(&local)
                    .filter(|((_, value), local)| local.as_deref() != Some(*value))
                    .map(|((key, value), _)| (*key, value.to_owned()))
                    .collect();
                // A preview spanning an external settings edit must never appear ready.
                if local_files::read_config(&path)? != observed {
                    return refused(GitFlowFailure::StalePreview);
                }
                Ok(GitFlowInitPreview {
                    repository: self.info().common_dir.clone(),
                    config: config.clone(),
                    production,
                    develop,
                    changes,
                    observed,
                })
            })
    }

    /// Create the development branch if needed, then write the reviewed settings in one
    /// config.lock transaction. Revalidated under the write gate; no switch or push.
    pub fn apply_gitflow_init(
        &self,
        preview: &GitFlowInitPreview,
        cancel: &CancellationToken,
    ) -> Result<GitFlowInitReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("gitflow-init", "revalidate_gitflow_init", || {
                if preview.repository != self.info().common_dir
                    || self.preview_gitflow_init(&preview.config, cancel)? != *preview
                {
                    return refused(GitFlowFailure::StalePreview);
                }
                cancel.check()
            })?;
        let develop = &preview.config.develop;
        let branch = match &preview.develop {
            GitFlowDevelop::Create { target, .. } => {
                let name = self.branch_argument(develop.as_bytes(), cancel)?;
                Some(self.create_branch_at(name, target, cancel)?)
            }
            GitFlowDevelop::Exists(_) => None,
        };
        let created = branch.as_ref().is_none_or(|b| b.write.result.is_ok());
        let mut edits: Vec<Vec<OsString>> = preview
            .changes
            .iter()
            .map(|(key, value)| args(&["--replace-all", key, value]))
            .collect();
        if let GitFlowDevelop::Create { track: true, .. } = &preview.develop {
            edits.push(args(&[
                "--replace-all",
                &format!("branch.{develop}.remote"),
                "origin",
            ]));
            edits.push(args(&[
                "--replace-all",
                &format!("branch.{develop}.merge"),
                &format!("refs/heads/{develop}"),
            ]));
        }
        let config = (created && !edits.is_empty()).then(|| {
            self.runner.observe("write_result", "gitflow-init", || {
                self.rewrite_local_config(&preview.observed, || cancel.check(), &edits, cancel)
            })
        });
        // Never reuse the caller's possibly cancelled token for reconciliation.
        let state = self.runner.observe("refresh", "gitflow-read", || {
            self.gitflow_state(&CancellationToken::new())
        });
        Ok(GitFlowInitReport {
            branch,
            config,
            state,
        })
    }

    fn gitflow_values(
        &self,
        local: bool,
        cancel: &CancellationToken,
    ) -> Result<[Option<String>; 7]> {
        let mut arguments = args(&["config"]);
        if local {
            arguments.extend(args(&["--local", "--no-includes"]));
        }
        arguments.extend(args(&["--null", "--get-regexp", KEY_PATTERN]));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        let mut values: [Option<String>; 7] = Default::default();
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(values);
        }
        let bytes = output.success("read git-flow settings")?;
        for entry in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            // A key written without `=` has no value line; git-flow reads it as empty.
            let (key, value) = match entry.iter().position(|b| *b == b'\n') {
                Some(i) => (&entry[..i], &entry[i + 1..]),
                None => (entry, &b""[..]),
            };
            let Some(index) = GITFLOW_KEYS.iter().position(|k| k.as_bytes() == key) else {
                continue;
            };
            let value = String::from_utf8(value.to_vec()).map_err(|_| {
                Error::Unsupported("Non-UTF-8 git-flow settings must be edited with Git".into())
            })?;
            // Later values win, as with `git config --get`.
            values[index] = Some(value);
        }
        Ok(values)
    }

    /// Ask Git whether the configured names make valid branch and tag names.
    fn check_gitflow_names(
        &self,
        config: &GitFlowConfig,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let invalid = |e: Error| match e {
            Error::InvalidInput(_) => Error::GitFlow(GitFlowFailure::InvalidConfig),
            other => other,
        };
        for name in [&config.master, &config.develop] {
            self.branch_argument(name.as_bytes(), cancel)
                .map_err(invalid)?;
        }
        for kind in GitFlowKind::ALL {
            let sample = config.branch_name(kind, "name");
            self.branch_argument(sample.as_bytes(), cancel)
                .map_err(invalid)?;
        }
        if !self.gitflow_tag_valid(&config.tag_name("1.0"), cancel)? {
            return refused(GitFlowFailure::InvalidConfig);
        }
        Ok(())
    }

    fn gitflow_tag_valid(&self, name: &str, cancel: &CancellationToken) -> Result<bool> {
        if name.is_empty() || name.starts_with('-') || name.contains('\0') {
            return Ok(false);
        }
        let output = self.runner.run(
            self.cwd(),
            &args(&["check-ref-format", &format!("refs/tags/{name}")]),
            vec![],
            cancel,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => output.success("validate tag name").map(|_| false),
        }
    }

    /// The commit an existing tag names, or `None` when the tag does not exist.
    fn gitflow_tag_commit(
        &self,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<ObjectId>> {
        let reference = format!("refs/tags/{name}");
        let exists = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", &reference]),
            vec![],
            cancel,
        )?;
        match exists.status.code() {
            Some(2) => Ok(None),
            Some(0) => self.resolve_commit(&reference, cancel).map(Some),
            _ => exists.success("inspect version tag").map(|_| None),
        }
    }

    fn gitflow_ancestor(
        &self,
        ancestor: &ObjectId,
        descendant: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        if ancestor == descendant {
            return Ok(true);
        }
        let output = self.runner.run(
            self.cwd(),
            &args(&[
                "merge-base",
                "--is-ancestor",
                ancestor.as_str(),
                descendant.as_str(),
            ]),
            vec![],
            cancel,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => output.success("git-flow ancestry").map(|_| false),
        }
    }

    /// Commits only on `local` and only on `origin/<name>`, when that remote-tracking
    /// branch exists. git-flow refuses a branch that is behind or diverged from it.
    fn gitflow_origin(
        &self,
        branches: &[Branch],
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<(u64, u64)>> {
        let (Some(local), Some(remote)) =
            (local_branch(branches, name), origin_branch(branches, name))
        else {
            return Ok(None);
        };
        if local.target == remote.target {
            return Ok(Some((0, 0)));
        }
        let range = format!("{}...{}", local.target, remote.target);
        let counts = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-list", "--left-right", "--count", &range, "--"]),
                vec![],
                cancel,
            )?
            .success("git-flow remote comparison")?;
        let counts: Vec<u64> = std::str::from_utf8(&counts)
            .ok()
            .into_iter()
            .flat_map(str::split_whitespace)
            .filter_map(|n| n.parse().ok())
            .collect();
        match counts[..] {
            [ahead, behind] => Ok(Some((ahead, behind))),
            _ => Err(Error::parse(
                "git-flow remote comparison",
                "expected two commit counts",
            )),
        }
    }

    /// `rev-list --count base..tip`: commits on `tip` that `base` lacks.
    fn gitflow_count(
        &self,
        base: &ObjectId,
        tip: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let range = format!("{base}..{tip}");
        let bytes = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-list", "--count", &range, "--"]),
                vec![],
                cancel,
            )?
            .success("git-flow commit count")?;
        std::str::from_utf8(crate::parse::line(&bytes))
            .ok()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| Error::parse("git-flow commit count", "expected a number"))
    }
}

fn local_branch<'a>(branches: &'a [Branch], name: &str) -> Option<&'a Branch> {
    branches.iter().find(|b| {
        b.kind == BranchKind::Local
            && b.symbolic_target.is_none()
            && b.reference.strip_prefix(b"refs/heads/") == Some(name.as_bytes())
    })
}
fn origin_branch<'a>(branches: &'a [Branch], name: &str) -> Option<&'a Branch> {
    branches.iter().find(|b| {
        b.kind == BranchKind::Remote
            && b.symbolic_target.is_none()
            && b.reference.strip_prefix(b"refs/remotes/origin/") == Some(name.as_bytes())
    })
}
/// A branch name for a Git argument after validation, or the typed refusal.
fn branch_os(repository: &Repository, name: &str, cancel: &CancellationToken) -> Result<OsString> {
    repository
        .branch_argument(name.as_bytes(), cancel)
        .map_err(|e| match e {
            Error::InvalidInput(_) => Error::GitFlow(GitFlowFailure::InvalidName),
            other => other,
        })
}
