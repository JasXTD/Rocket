use crate::{
    parse,
    runner::{Runner, args, find_git, git_path},
    *,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct GitClient {
    pub(crate) runner: Runner,
    version: String,
}

impl GitClient {
    // An already validated runner can discover related repositories without
    // repeating executable/version probing. This client is private to the backend.
    pub(crate) fn from_runner(runner: Runner) -> Self {
        Self {
            runner,
            version: String::new(),
        }
    }
    /// Locate the bundled runtime when included, otherwise installed Git on PATH.
    /// This only resolves a path; every client still verifies it before execution.
    pub fn automatic_executable() -> Result<PathBuf> { find_git() }

    /// Resolve the first Git on PATH without selecting or executing the bundle.
    pub fn installed_executable() -> Result<PathBuf> { crate::runner::find_installed_git() }

    pub fn discover() -> Result<Self> {
        Self::new(find_git()?, CommandLimits::default())
    }

    pub fn new(executable: impl AsRef<Path>, limits: CommandLimits) -> Result<Self> {
        Self::new_inner(executable.as_ref(), limits, None, &CancellationToken::new())
    }

    /// Discover bundled/installed Git (or use an explicit executable), with the caller's
    /// limits and cancellation also applied to its initial version probe.
    pub fn discover_with_options(
        executable: Option<&Path>,
        limits: CommandLimits,
        log: Option<DiagnosticLog>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        cancel.check()?;
        let executable = match executable {
            Some(path) => path.to_path_buf(),
            None => match &log {
                Some(log) => log.step("discover", "locate_git", find_git)?,
                None => find_git()?,
            },
        };
        Self::new_inner(&executable, limits, log, cancel)
    }

    /// Discover Git and record sanitized command diagnostics in an explicit logger.
    pub fn discover_with_log(log: DiagnosticLog) -> Result<Self> {
        let executable = log.step("discover", "locate_git", find_git)?;
        Self::new_inner(
            &executable,
            CommandLimits::default(),
            Some(log),
            &CancellationToken::new(),
        )
    }

    pub fn new_with_log(
        executable: impl AsRef<Path>,
        limits: CommandLimits,
        log: DiagnosticLog,
    ) -> Result<Self> {
        Self::new_inner(
            executable.as_ref(),
            limits,
            Some(log),
            &CancellationToken::new(),
        )
    }

    fn new_inner(
        executable: &Path,
        limits: CommandLimits,
        log: Option<DiagnosticLog>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        match log.clone() {
            Some(context) => context.step("discover", "verify_git", || {
                Self::new_checked(executable, limits, log, cancel)
            }),
            None => Self::new_checked(executable, limits, log, cancel),
        }
    }

    fn new_checked(
        executable: &Path,
        limits: CommandLimits,
        log: Option<DiagnosticLog>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        cancel.check()?;
        // Classify the requested path before canonicalizing, so a replaced
        // bundled executable cannot escape verification through a leaf link.
        let managed_git = crate::managed_git::for_executable(executable, cancel, log.as_ref())?;
        let executable = executable
            .canonicalize()
            .map_err(|e| Error::io("resolve Git executable", e))?;
        if !executable.is_file() {
            return Err(Error::InvalidInput("Git executable must be a file".into()));
        }
        let mut runner = Runner {
            executable,
            limits,
            log,
            transport_scope: None,
            secret_protection: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            guard_ai_api_keys: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            push_check: Default::default(),
            git_http_allowed: !cfg!(windows),
            managed_git,
        };
        let cwd = std::env::current_dir().map_err(|e| Error::io("read current directory", e))?;
        let version = runner
            .run(&cwd, &args(&["--version"]), Vec::new(), cancel)?
            .success("version")?;
        let version = std::str::from_utf8(parse::line(&version))
            .map_err(|_| Error::parse("version", "non-UTF-8 version"))?
            .to_owned();
        let components: Vec<_> = version
            .strip_prefix("git version ")
            .unwrap_or("")
            .split('.')
            .collect();
        let major = components.first().and_then(|v| v.parse::<u32>().ok());
        let minor = components.get(1).and_then(|v| v.parse::<u32>().ok());
        if let (Some(log), Some(major), Some(minor)) = (&runner.log, major, minor) {
            log.git_version(
                major,
                minor,
                components.get(2).and_then(|v| v.parse::<u32>().ok()),
            );
        }
        if !matches!((major, minor), (Some(2), Some(46..)) | (Some(3..), Some(_))) {
            return Err(Error::Unsupported(format!(
                "Git 2.46 or later required; found {version}"
            )));
        }
        runner.git_http_allowed = !cfg!(windows)
            || crate::runner::http_safety::patched_windows_git(&version);
        Ok(Self { runner, version })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
    /// An affected/unknown Windows Git runtime can still serve local and SSH work.
    pub fn http_transport_warning(&self) -> Option<&'static str> {
        (!self.runner.git_http_allowed).then_some(crate::runner::http_safety::WARNING)
    }
    pub fn executable(&self) -> &Path {
        &self.runner.executable
    }
    pub fn is_managed(&self) -> bool {
        self.runner.managed_git.is_some()
    }

    /// Reuse the verified executable and limits with a fresh diagnostic context.
    pub fn with_log(&self, log: DiagnosticLog) -> Self {
        let mut client = self.clone();
        client.runner.log = Some(log);
        client
    }

    /// Limit every transport launched by this client, including quiet fetches.
    /// Normal desktop/CLI clients remain unrestricted unless explicitly scoped.
    pub fn with_transport_scope(mut self, scope: Option<TransportScope>) -> Self {
        self.runner.transport_scope = scope;
        self
    }

    /// Local application preference. Enabled on every newly constructed client;
    /// shared clones observe explicit user changes. No repository setting can disable it.
    pub fn set_secret_protection(&self, enabled: bool) {
        self.runner.secret_protection.store(enabled, std::sync::atomic::Ordering::Release);
    }

    /// Independent, default-on AI API-key format guard. Shared clones and
    /// already-open repositories observe the application's explicit preference.
    pub fn set_guard_ai_api_keys(&self, enabled: bool) {
        self.runner.guard_ai_api_keys.store(enabled, std::sync::atomic::Ordering::Release);
    }

    /// Optional application-owned check shared with existing and request-specific clients.
    pub fn set_push_check(&self, check: Option<Arc<dyn crate::PushCheck>>) -> Result<()> {
        self.runner.push_check.set(check)
    }

    /// Preserve the execution policy and recheck Git for a request-specific client.
    pub fn for_request(&self, limits: CommandLimits, log: DiagnosticLog) -> Result<Self> {
        let mut client = Self::new_with_log(&self.runner.executable, limits, log)?
            .with_transport_scope(self.runner.transport_scope.clone());
        client.runner.secret_protection = self.runner.secret_protection.clone();
        client.runner.guard_ai_api_keys = self.runner.guard_ai_api_keys.clone();
        client.runner.push_check = self.runner.push_check.clone();
        Ok(client)
    }

    /// Discover a repository from its worktree (including a subdirectory) or bare directory.
    /// All identity paths come from Git, rather than assuming `.git` is a directory.
    pub fn open(
        &self,
        directory: impl AsRef<Path>,
        cancel: &CancellationToken,
    ) -> Result<Repository> {
        self.runner.step("open", "repository_discovery", || {
            self.open_inner(directory.as_ref(), cancel)
        })
    }

    fn open_inner(&self, directory: &Path, cancel: &CancellationToken) -> Result<Repository> {
        let directory = directory
            .canonicalize()
            .map_err(|e| Error::io("resolve repository directory", e))?;
        if !directory.is_dir() {
            return Err(Error::InvalidInput(
                "repository location must be a directory".into(),
            ));
        }
        let read = |options: &[&str]| -> Result<Vec<u8>> {
            self.runner
                .run(&directory, &args(options), Vec::new(), cancel)?
                .success("discovery")
        };
        // One Git startup for the common case. Paths can contain newlines, so
        // only use a six-record result; ambiguous paths use individual queries.
        // Bare repositories reject --show-toplevel and use that same fallback.
        let probe = self.runner.run(
            &directory,
            &args(&[
                "rev-parse",
                "--path-format=absolute",
                "--is-bare-repository",
                "--show-object-format=storage",
                "--is-shallow-repository",
                "--absolute-git-dir",
                "--git-common-dir",
                "--show-toplevel",
            ]),
            Vec::new(),
            cancel,
        )?;
        if probe.status.success() {
            let fields: Vec<_> = parse::line(&probe.stdout).split(|b| *b == b'\n').collect();
            if fields.len() == 6 && fields[0] == b"false" {
                let info = RepositoryInfo {
                    worktree: Some(absolute_path(fields[5])?),
                    git_dir: absolute_path(fields[3])?,
                    common_dir: absolute_path(fields[4])?,
                    object_format: object_format(fields[1])?,
                    is_shallow: boolean(fields[2])?,
                };
                cancel.check()?;
                return Ok(Repository {
                    runner: self.runner.clone(),
                    write_gate: crate::write_gate::for_repository(&info.common_dir)?,
                    info,
                });
            }
        }
        let git_dir = absolute_path(&read(&["rev-parse", "--absolute-git-dir"])?)?;
        let common_dir = absolute_path(&read(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])?)?;
        let bare = boolean(&read(&["rev-parse", "--is-bare-repository"])?)?;
        let worktree = if bare {
            None
        } else {
            Some(absolute_path(&read(&[
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
            ])?)?)
        };
        let object_format = object_format(parse::line(&read(&[
            "rev-parse",
            "--show-object-format=storage",
        ])?))?;
        let is_shallow = boolean(&read(&["rev-parse", "--is-shallow-repository"])?)?;
        cancel.check()?;
        let write_gate = crate::write_gate::for_repository(&common_dir)?;
        Ok(Repository {
            runner: self.runner.clone(),
            write_gate,
            info: RepositoryInfo {
                worktree,
                git_dir,
                common_dir,
                object_format,
                is_shallow,
            },
        })
    }
}

fn absolute_path(bytes: &[u8]) -> Result<PathBuf> {
    let path = parse::path(parse::line(bytes))?;
    if !path.is_absolute() {
        return Err(Error::parse("discovery", "expected an absolute path"));
    }
    path.canonicalize()
        .map_err(|e| Error::io("resolve repository identity", e))
}

fn boolean(bytes: &[u8]) -> Result<bool> {
    match parse::line(bytes) {
        b"true" => Ok(true),
        b"false" => Ok(false),
        _ => Err(Error::parse("discovery", "expected a boolean")),
    }
}

fn object_format(bytes: &[u8]) -> Result<ObjectFormat> {
    match bytes {
        b"sha1" => Ok(ObjectFormat::Sha1),
        b"sha256" => Ok(ObjectFormat::Sha256),
        _ => Err(Error::Unsupported(
            "unknown repository object format".into(),
        )),
    }
}

/// Opaque effective Git configuration for detecting changes across a read-only
/// review. Values can contain secrets: never format, log or persist this snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigurationSnapshot(Vec<u8>);

impl ConfigurationSnapshot {
    /// Remotes from this exact effective-configuration read, including repeated
    /// values and includes. The rest of the configuration remains opaque.
    pub fn remotes(&self, cancel: &CancellationToken) -> Result<Vec<Remote>> {
        cancel.check()?;
        if self.0.is_empty() {
            return Ok(Vec::new());
        }
        let records = self.0.strip_suffix(b"\0")
            .ok_or_else(|| Error::parse("configuration snapshot", "unterminated record"))?;
        let mut fields = records.split(|&byte| byte == 0);
        let mut remotes = Vec::new();
        while let Some(scope) = fields.next() {
            cancel.check()?;
            let origin = fields.next()
                .ok_or_else(|| Error::parse("configuration snapshot", "missing origin"))?;
            let record = fields.next()
                .ok_or_else(|| Error::parse("configuration snapshot", "missing value"))?;
            if scope.is_empty() || origin.is_empty() {
                return Err(Error::parse("configuration snapshot", "empty scope or origin"));
            }
            let key = record.split(|&byte| byte == b'\n').next().unwrap_or_default();
            if key.starts_with(b"remote.")
                && [b".url".as_slice(), b".pushurl", b".fetch"].iter()
                    .any(|suffix| key.len() >= b"remote.".len() + suffix.len()
                        && key.ends_with(suffix))
            {
                remotes.extend_from_slice(record);
                remotes.push(0);
            }
        }
        let remotes = crate::remote::parse_remotes(&remotes)?;
        cancel.check()?;
        Ok(remotes)
    }
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub(crate) runner: Runner,
    info: RepositoryInfo,
    pub(crate) write_gate: Arc<crate::write_gate::WriteGate>,
}

impl Repository {
    /// Clone this open handle with a new diagnostic request context. Discovery,
    /// limits and repository identity are retained, but no Git output is cached.
    pub fn with_log(&self, log: DiagnosticLog) -> Self {
        let mut repository = self.clone();
        repository.runner.log = Some(log);
        repository
    }

    /// Includes system/global/local/worktree configuration and includes as Git
    /// resolves them. Call on a worker; no transport or credential helper runs.
    pub fn configuration_snapshot(
        &self,
        cancel: &CancellationToken,
    ) -> Result<ConfigurationSnapshot> {
        self.read_diff(
            "inspect effective configuration",
            &args(&[
                "config",
                "--null",
                "--list",
                "--show-origin",
                "--show-scope",
            ]),
            cancel,
        )
        .map(ConfigurationSnapshot)
    }

    pub fn info(&self) -> &RepositoryInfo {
        &self.info
    }

    pub(crate) fn cwd(&self) -> &Path {
        self.info.worktree.as_deref().unwrap_or(&self.info.git_dir)
    }

    fn read(
        &self,
        operation: &'static str,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), arguments, input, cancel)?
            .success(operation)
    }

    pub fn status(&self, cancel: &CancellationToken) -> Result<Status> {
        if self.info.worktree.is_none() {
            return Err(Error::WorktreeRequired);
        }
        let output = self.read_diff(
            "status",
            &args(&[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--ignore-submodules=none",
                "--renames",
            ]),
            cancel,
        )?;
        let status = parse::status(&output)?;
        cancel.check()?;
        Ok(status)
    }

    pub fn head(&self, cancel: &CancellationToken) -> Result<Head> {
        // Resolve the commit before switching output to its full symbolic name.
        // Both revisions are constant. This keeps HEAD^{commit} validation and
        // avoids enumerating unrelated loose refs just to read HEAD.
        let summary = self.runner.run(
            self.cwd(),
            &args(&[
                "rev-parse",
                "--revs-only",
                "HEAD^{commit}",
                "--symbolic-full-name",
                "HEAD",
            ]),
            Vec::new(),
            cancel,
        );
        match summary {
            Ok(output) if output.status.success() && !output.stdout.is_empty() => {
                // Two lines are expected. Bound allocation for malformed output.
                let fields: Vec<_> = parse::line(&output.stdout)
                    .split(|&byte| byte == b'\n')
                    .take(3)
                    .collect();
                if let [target, reference] = fields.as_slice()
                    && (*reference == b"HEAD" || reference.starts_with(b"refs/"))
                {
                    let target = parse::oid(target, "head")?;
                    cancel.check()?;
                    return Ok(if *reference == b"HEAD" {
                        Head::Detached { target }
                    } else {
                        Head::Branch {
                            name: reference
                                .strip_prefix(b"refs/heads/")
                                .unwrap_or(reference)
                                .to_vec(),
                            target,
                        }
                    });
                }
                // Unresolved or unusual symbolic names retain the exact
                // existence checks and error behavior below.
            }
            Ok(_) | Err(Error::OutputLimit { .. }) => {
                // Exact reads retain existing error/peeling behavior and fit
                // smaller output caps.
            }
            Err(error) => return Err(error),
        }
        let symbolic = self.runner.run(
            self.cwd(),
            &args(&["symbolic-ref", "--quiet", "HEAD"]),
            Vec::new(),
            cancel,
        )?;
        let reference = match symbolic.status.code() {
            Some(0) => Some(parse::line(&symbolic.stdout).to_vec()),
            Some(1) => None, // Detached HEAD is a documented exit status, not an error string.
            _ => return symbolic.success("head").map(|_| unreachable!()),
        };
        if let Some(reference) = &reference {
            let mut arguments = args(&["show-ref", "--exists", "--"]);
            arguments.push(crate::model::bytes_to_os(reference)?);
            let exists = self
                .runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?;
            match exists.status.code() {
                Some(2) => {
                    return Ok(Head::Unborn {
                        branch: reference
                            .strip_prefix(b"refs/heads/")
                            .unwrap_or(reference)
                            .to_vec(),
                    });
                }
                Some(0) => {}
                _ => return exists.success("head").map(|_| unreachable!()),
            }
        }
        let target = self.resolve_commit("HEAD", cancel)?;
        Ok(match reference {
            Some(reference) => Head::Branch {
                name: reference
                    .strip_prefix(b"refs/heads/")
                    .unwrap_or(&reference)
                    .to_vec(),
                target,
            },
            None => Head::Detached { target },
        })
    }

    pub fn branches(&self, cancel: &CancellationToken) -> Result<Vec<Branch>> {
        let arguments = args(&[
            "for-each-ref",
            "--sort=refname",
            // The last two fields: the tip's commit time and how far the branch is
            // from its upstream ("ahead 1, behind 3", "gone" or empty when level).
            "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream)%00%(symref)%00%(worktreepath)%00%(committerdate:unix)%00%(upstream:track,nobracket)%00",
            "refs/heads/",
            "refs/remotes/",
        ]);
        let output = self.read("branches", &arguments, Vec::new(), cancel)?;
        let branches = parse::branches(&output)?;
        cancel.check()?;
        Ok(branches)
    }

    pub fn resolve_commit(&self, revision: &str, cancel: &CancellationToken) -> Result<ObjectId> {
        let arguments = commit_revision_args(revision)?;
        let output = self.read("resolve commit", &arguments, Vec::new(), cancel)?;
        parse::oid(parse::line(&output), "resolve commit")
    }

    /// Topological history paged against immutable starting commits.
    /// Later pages currently retraverse with --skip; output/memory stay bounded, but deep
    /// scrolling costs more CPU. A persistent traversal can replace this after profiling.
    pub fn history(
        &self,
        start: HistoryStart,
        page_size: usize,
        cancel: &CancellationToken,
    ) -> Result<HistoryPage> {
        cancel.check()?;
        if !(1..=1000).contains(&page_size) {
            return Err(Error::InvalidInput(
                "history page size must be 1..1000".into(),
            ));
        }
        self.history_batch(start, page_size, cancel)
    }

    /// Begin an ancestry-safe ordered traversal; continuing a cursor must retain its order.
    pub fn history_ordered(&self, start: HistoryStart, page_size: usize, order: HistoryOrder,
        cancel: &CancellationToken) -> Result<HistoryPage> {
        cancel.check()?;
        if !(1..=1000).contains(&page_size) {
            return Err(Error::InvalidInput("history page size must be 1..1000".into()));
        }
        self.history_batch_ordered(start, page_size, order, cancel)
    }

    /// The search worker may scan a larger, still bounded batch. Public history
    /// pages retain their 1,000-row limit; no traversal or object data is cached.
    pub(crate) fn history_batch(
        &self,
        start: HistoryStart,
        page_size: usize,
        cancel: &CancellationToken,
    ) -> Result<HistoryPage> {
        let order = match &start {
            HistoryStart::Continue(c) => c.order,
            HistoryStart::OrderedTips(_, order) => *order,
            _ => HistoryOrder::default(),
        };
        self.history_batch_ordered(start, page_size, order, cancel)
    }
    fn history_batch_ordered(&self, start: HistoryStart, page_size: usize, order: HistoryOrder,
        cancel: &CancellationToken) -> Result<HistoryPage> {
        cancel.check()?;
        if !(1..=4096).contains(&page_size) {
            return Err(Error::InvalidInput("history batch must be 1..4096".into()));
        }
        let (mut tips, offset) = match start {
            HistoryStart::Commit(id) => (vec![id], 0),
            HistoryStart::Tips(ids) => (ids, 0),
            HistoryStart::OrderedTips(ids, pinned) => {
                if pinned != order { return Err(Error::InvalidInput("history order changed; start a new traversal".into())); }
                (ids, 0)
            }
            HistoryStart::AllBranches => {
                let mut ids: Vec<_> = self
                    .branches(cancel)?
                    .into_iter()
                    .map(|b| b.target)
                    .collect();
                if let Head::Branch { target, .. } | Head::Detached { target } =
                    self.head(cancel)?
                {
                    ids.push(target);
                }
                (ids, 0)
            }
            HistoryStart::Head => match self.head(cancel)? {
                Head::Unborn { .. } => {
                    return Ok(HistoryPage { order, ..Default::default() });
                }
                Head::Branch { target, .. } | Head::Detached { target } => (vec![target], 0),
            },
            HistoryStart::Revision(revision) => (vec![self.resolve_commit(&revision, cancel)?], 0),
            HistoryStart::Continue(cursor) => {
                if cursor.order != order {
                    return Err(Error::InvalidInput("history cursor order changed; start a new traversal".into()));
                }
                if cursor.repository != self.info.git_dir {
                    return Err(Error::InvalidInput(
                        "history cursor belongs to a different repository/worktree".into(),
                    ));
                }
                (cursor.tips, cursor.offset)
            }
        };
        tips.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        tips.dedup();
        if tips.is_empty() {
            return Ok(HistoryPage { order, ..Default::default() });
        }
        if tips.len() > 100_000 {
            return Err(Error::Unsupported(
                "history has more than 100,000 starting tips; choose a branch".into(),
            ));
        }
        let arguments = args(&[
            "rev-list",
            order.argument(),
            &format!("--max-count={}", page_size + 1),
            &format!("--skip={offset}"),
            "--stdin",
            "--",
        ]);
        // stdin avoids the Windows command-line size limit for many branches.
        // Only validated ObjectIds become revision expressions, never user text.
        let input = tips
            .iter()
            .flat_map(|id| format!("{}^{{commit}}\n", id.as_str()).into_bytes())
            .collect();
        let output = self.read("history traversal", &arguments, input, cancel)?;
        let mut ids = Vec::new();
        for value in output
            .split(|&b| b == b'\n')
            .filter(|value| !value.is_empty())
        {
            ids.push(parse::oid(value, "history traversal")?);
        }
        let more = ids.len() > page_size;
        ids.truncate(page_size);
        if ids.is_empty() {
            return Ok(HistoryPage {
                commits: Vec::new(),
                next: None,
                tips,
                order,
            });
        }
        let input: Vec<u8> = ids
            .iter()
            .flat_map(|id| id.as_str().bytes().chain(std::iter::once(b'\n')))
            .collect();
        let output = self.read(
            "history objects",
            &args(&["cat-file", "--batch", "--buffer"]),
            input,
            cancel,
        )?;
        let commits = parse::commits(&output, &ids)?;
        let next = if more {
            Some(HistoryCursor {
                repository: self.info.git_dir.clone(),
                tips: tips.clone(),
                order,
                offset: offset
                    .checked_add(commits.len())
                    .ok_or_else(|| Error::InvalidInput("history cursor overflow".into()))?,
            })
        } else {
            None
        };
        cancel.check()?;
        Ok(HistoryPage {
            commits,
            next,
            tips,
            order,
        })
    }

    pub fn diff(&self, options: &DiffOptions, cancel: &CancellationToken) -> Result<Diff> {
        let arguments = self.diff_arguments(options, cancel)?;
        let patch = self.read_diff("diff", &arguments, cancel)?;
        cancel.check()?;
        Ok(Diff { patch })
    }

    pub(crate) fn diff_arguments(
        &self,
        options: &DiffOptions,
        cancel: &CancellationToken,
    ) -> Result<Vec<OsString>> {
        cancel.check()?;
        if self.info.worktree.is_none()
            && !matches!(
                options.target,
                DiffTarget::Between { .. } | DiffTarget::Commit { .. }
            )
        {
            return Err(Error::WorktreeRequired);
        }
        let mut arguments = args(&[
            "diff",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--ignore-submodules=none",
            "--submodule=short",
            "--find-renames",
            "--no-exit-code",
            &format!("--unified={}", options.context_lines),
        ]);
        if matches!(options.target, DiffTarget::Commit { .. }) {
            arguments[0] = "show".into();
            arguments.extend(args(&[
                "--format=",
                "--first-parent",
                "--root",
                "--no-show-signature",
                "--no-notes",
                "--no-decorate",
            ]));
        }
        let resolve = |revision: &str| -> Result<ObjectId> {
            let output =
                self.read_diff("resolve commit", &commit_revision_args(revision)?, cancel)?;
            parse::oid(parse::line(&output), "resolve commit")
        };
        match &options.target {
            DiffTarget::Unstaged => {}
            DiffTarget::Staged => arguments.push("--cached".into()),
            DiffTarget::Local { base, staged } => {
                if *staged {
                    arguments.push("--cached".into());
                }
                arguments.push(resolve(base)?.as_str().into());
            }
            DiffTarget::Between { base, target } => {
                arguments.push(resolve(base)?.as_str().into());
                arguments.push(resolve(target)?.as_str().into());
            }
            DiffTarget::Commit { id } => {
                // ObjectId is already a full hexadecimal ID, so it cannot be an
                // option. Let show verify/peel it to a commit in the same process.
                arguments.push(format!("{}^{{commit}}", id.as_str()).into());
            }
        }
        arguments.push("--".into());
        for path in &options.paths {
            arguments.push(path.argument()?);
        }
        Ok(arguments)
    }

    pub(crate) fn read_diff(
        &self,
        operation: &'static str,
        arguments: &[OsString],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.read_diff_input(operation, arguments, Vec::new(), cancel)
    }

    pub(crate) fn read_diff_input(
        &self,
        operation: &'static str,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        // A retained handle must not discover an ancestor repository if its .git
        // marker disappears, or follow a changed linked-worktree marker elsewhere.
        let mut bound = args(&["--git-dir"]);
        bound.push(git_path(&self.info.git_dir));
        if let Some(worktree) = &self.info.worktree {
            bound.push("--work-tree".into());
            bound.push(git_path(worktree));
        }
        bound.extend_from_slice(arguments);
        self.read(operation, &bound, input, cancel)
    }
}

pub(crate) fn commit_revision_args(revision: &str) -> Result<Vec<OsString>> {
    if revision.is_empty() || revision.contains('\0') || revision.len() > 4096 {
        return Err(Error::InvalidInput(
            "revision must contain 1..4096 bytes without NUL".into(),
        ));
    }
    let mut arguments = args(&["rev-parse", "--verify", "--end-of-options"]);
    arguments.push(format!("{revision}^{{commit}}").into());
    Ok(arguments)
}

#[cfg(test)]
mod loading_tests {
    use super::*;

    #[test]
    fn configuration_remotes_preserve_order_bytes_and_empty_values() {
        let snapshot = ConfigurationSnapshot(
            b"global\0file:global\0remote.Work.foo.url\nfirst\nline\0\
              local\0file:local\0remote.Work.foo.url\nsecond\0\
              worktree\0file:worktree\0remote.Work.foo.pushurl\npush\0\
              command\0command line:\0remote.Work.foo.fetch\n+refs/heads/*:refs/remotes/Work.foo/*\0\
              local\0file:local\0remote.empty.url\0\
              local\0file:local\0remote.url\nnot a named remote\0\
              local\0file:local\0remote.Work.foo.prune\ntrue\0\
              local\0file:local\0unrelated.key\nremote.fake.url\nignored\0".to_vec(),
        );
        let remotes = snapshot.remotes(&CancellationToken::new()).unwrap();
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, b"Work.foo");
        assert_eq!(remotes[0].urls, [b"first\nline".to_vec(), b"second".to_vec()]);
        assert_eq!(remotes[0].push_urls, [b"push".to_vec()]);
        assert_eq!(remotes[0].fetch_refspecs, [b"+refs/heads/*:refs/remotes/Work.foo/*".to_vec()]);
        assert_eq!(remotes[1].name, b"empty");
        assert_eq!(remotes[1].urls, [Vec::<u8>::new()]);
    }

    #[test]
    fn configuration_remotes_reject_broken_framing_and_honor_cancellation() {
        for bytes in [
            b"local\0file:x\0remote.x.url\nx".as_slice(),
            b"local\0file:x\0",
            b"local\0",
            b"\0file:x\0remote.x.url\nx\0",
            b"local\0\0remote.x.url\nx\0",
            b"local\0file:x\0remote..url\nx\0",
        ] {
            assert!(ConfigurationSnapshot(bytes.to_vec()).remotes(&CancellationToken::new()).is_err());
        }
        let empty = ConfigurationSnapshot(Vec::new());
        assert!(empty.remotes(&CancellationToken::new()).unwrap().is_empty());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(empty.remotes(&cancel), Err(Error::Cancelled)));
    }
}
