//! Content-only replay in disposable object storage. No index, worktree, ref,
//! hook, signing program, filter, custom merge driver or network operation runs.
use crate::{
    parse,
    runner::{Runner, args},
    *,
};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum ReplayKind<'a> {
    Commit(&'a CommitActionRequest),
    Rebase { attributes_changing: bool },
}
impl ReplayKind<'_> {
    fn operation(self) -> &'static str {
        match self {
            Self::Commit(_) => "commit-prediction",
            Self::Rebase { .. } => "rebase-prediction",
        }
    }
    fn stale(self) -> Error {
        match self {
            Self::Commit(_) => Error::CommitAction(CommitFailure::StalePreview),
            Self::Rebase { .. } => Error::Rebase(RebaseFailure::StalePreview),
        }
    }
}

const PATH_LIMIT: usize = 50_000;
const CHECK_BUDGET: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredictionKind {
    Clean,
    Conflicts,
    Empty,
    Unavailable,
}
impl PredictionKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Conflicts => "conflicts",
            Self::Empty => "empty",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredictionReason {
    MergeSettings,
    Attributes,
    AttributeChange,
    Submodules,
    ReplacedObjects,
    Limit,
    GitFailed,
    TimedOut,
}
impl PredictionReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::MergeSettings => "merge_settings",
            Self::Attributes => "merge_attributes",
            Self::AttributeChange => "attributes_changing",
            Self::Submodules => "submodules",
            Self::Limit => "prediction_limit",
            Self::GitFailed => "prediction_failed",
            Self::TimedOut => "prediction_timeout",
            Self::ReplacedObjects => "replaced_objects",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::MergeSettings => {
                "Custom merge settings are outside this check. No merge driver was run and no clean result is claimed."
            }
            Self::Attributes => {
                "Merge, filter or encoding attributes need the actual checkout. No external program was run and no clean result is claimed."
            }
            Self::AttributeChange => {
                "This sequence changes .gitattributes. Review those rules before applying; the check does not simulate changing attributes."
            }
            Self::Submodules => {
                "Submodule merge ancestry needs its own repository. The check does not predict submodule conflicts."
            }
            Self::ReplacedObjects => {
                "This repository replaces commit objects or ancestry. The isolated check does not simulate replacements or grafts."
            }
            Self::Limit => {
                "This check reached its size or time budget. Unchecked commits are not assumed clean."
            }
            Self::GitFailed => {
                "Git could not complete the isolated check. Inspect diagnostics and refresh, or review the change before starting the actual operation."
            }
            Self::TimedOut => {
                "The isolated Git check timed out. Refresh to check again; unchecked commits are not assumed clean."
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitPrediction {
    pub kind: PredictionKind,
    /// Number simulated, including the first conflicting/empty commit.
    pub checked: usize,
    pub total: usize,
    /// Zero-based first conflict/empty commit. Later commits have not been checked.
    pub stopped_at: Option<usize>,
    pub paths: Vec<RepoPath>,
    pub reason: Option<PredictionReason>,
}
impl CommitPrediction {
    fn unavailable(total: usize, reason: PredictionReason) -> Self {
        Self {
            kind: PredictionKind::Unavailable,
            checked: 0,
            total,
            stopped_at: None,
            paths: Vec::new(),
            reason: Some(reason),
        }
    }
}

/// Values are retained privately for freshness; never include them in diagnostic output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PredictionGuard {
    paths: Vec<Vec<u8>>,
    config: Vec<u8>,
    attributes: Vec<u8>,
    replacements: Vec<u8>,
    grafts: bool,
}
impl Repository {
    fn prediction_guard(
        &self,
        paths: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<PredictionGuard> {
        let out = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--null",
                "--get-regexp",
                "^(merge\\.|diff\\.(renames|renamelimit)$|core\\.attributesfile$)",
            ]),
            Vec::new(),
            cancel,
        )?;
        let config = if out.status.code() == Some(1) {
            Vec::new()
        } else {
            out.success("commit-prediction")?
        };
        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(path);
            input.push(0);
        }
        let attributes = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "check-attr",
                    "-z",
                    "--stdin",
                    "merge",
                    "filter",
                    "working-tree-encoding",
                ]),
                input,
                cancel,
            )?
            .success("commit-prediction")?;
        let replacements = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/replace/",
                ]),
                Vec::new(),
                cancel,
            )?
            .success("commit-prediction")?;
        let grafts = match fs::symlink_metadata(self.info().common_dir.join("info/grafts")) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(Error::io("inspect prediction grafts", e)),
        };
        Ok(PredictionGuard {
            paths: paths.to_vec(),
            config,
            attributes,
            replacements,
            grafts,
        })
    }
    pub(crate) fn prediction_is_current(
        &self,
        guard: &PredictionGuard,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        Ok(self.prediction_guard(&guard.paths, cancel)? == *guard)
    }
    pub(crate) fn validate_prediction(
        &self,
        guard: &PredictionGuard,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if !self.prediction_is_current(guard, cancel)? {
            return Err(Error::CommitAction(CommitFailure::StalePreview));
        }
        Ok(())
    }
    pub(crate) fn predict_commits(
        &self,
        request: &CommitActionRequest,
        head: &ObjectId,
        commits: &[Commit],
        affected: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<(CommitPrediction, Option<PredictionGuard>)> {
        self.predict_replay(ReplayKind::Commit(request), head, commits, affected, cancel)
    }
    pub(crate) fn predict_rebase(
        &self,
        onto: &ObjectId,
        commits: &[Commit],
        affected: &[Vec<u8>],
        attributes_changing: bool,
        cancel: &CancellationToken,
    ) -> Result<(CommitPrediction, Option<PredictionGuard>)> {
        self.predict_replay(
            ReplayKind::Rebase {
                attributes_changing,
            },
            onto,
            commits,
            affected,
            cancel,
        )
    }
    fn predict_replay(
        &self,
        kind: ReplayKind<'_>,
        head: &ObjectId,
        commits: &[Commit],
        affected: &[Vec<u8>],
        cancel: &CancellationToken,
    ) -> Result<(CommitPrediction, Option<PredictionGuard>)> {
        let total = commits.len();
        let operation = kind.operation();
        let result = self.runner.step(operation, "inspect_merge_context", || {
            let bytes = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["ls-tree", "-r", "-z", "--full-tree", head.as_str()]),
                    Vec::new(),
                    cancel,
                )?
                .success("commit-prediction")?;
            let (mut paths, submodule) = tree_paths(&bytes)?;
            paths.extend(affected.iter().cloned());
            paths.sort();
            paths.dedup();
            if paths.len() > PATH_LIMIT {
                return Ok((
                    CommitPrediction::unavailable(total, PredictionReason::Limit),
                    None,
                ));
            }
            let guard = self.prediction_guard(&paths, cancel)?;
            let attributes_changing = match kind {
                ReplayKind::Commit(_) => affected.iter().any(|p| is_attributes(p)),
                ReplayKind::Rebase {
                    attributes_changing,
                } => attributes_changing,
            };
            let reason = if attributes_changing {
                Some(PredictionReason::AttributeChange)
            } else if submodule {
                Some(PredictionReason::Submodules)
            } else {
                unsupported(&guard)?
            };
            if let Some(reason) = reason {
                return Ok((CommitPrediction::unavailable(total, reason), Some(guard)));
            }
            let scratch = patches::PreviewScratch::new()?;
            let outcome = self.runner.step(operation, "simulate_commits", || {
                self.simulate_commits(&scratch.path, kind, head, commits, &guard, cancel)
            });
            let cleanup = self
                .runner
                .step(operation, "cleanup_preview", || scratch.cleanup());
            // Explicit cleanup is observed even when cancelled. Drop is only the fallback.
            cleanup?;
            let (prediction, paths) = outcome?;
            if !self.prediction_is_current(&guard, cancel)? {
                return Err(kind.stale());
            }
            let final_guard = self.prediction_guard(&paths, cancel)?;
            let prediction = match unsupported(&final_guard)? {
                Some(reason) => CommitPrediction::unavailable(total, reason),
                None => prediction,
            };
            Ok((prediction, Some(final_guard)))
        });
        let result = match result {
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(
                e @ (Error::CommitAction(CommitFailure::StalePreview)
                | Error::Rebase(RebaseFailure::StalePreview)),
            ) => return Err(e),
            Err(Error::TimedOut) => (
                CommitPrediction::unavailable(total, PredictionReason::TimedOut),
                None,
            ),
            Err(_) => (
                CommitPrediction::unavailable(total, PredictionReason::GitFailed),
                None,
            ),
            Ok(result) => result,
        };
        cancel.check()?;
        if let Some(log) = &self.runner.log {
            log.replay_prediction(operation, &result.0);
        }
        Ok(result)
    }

    fn simulate_commits(
        &self,
        scratch: &Path,
        kind: ReplayKind<'_>,
        head: &ObjectId,
        commits: &[Commit],
        guard: &PredictionGuard,
        cancel: &CancellationToken,
    ) -> Result<(CommitPrediction, Vec<Vec<u8>>)> {
        let empty = scratch.join("empty.config");
        fs::write(&empty, b"").map_err(|e| Error::io("prepare prediction configuration", e))?;
        let format = match self.info().object_format {
            ObjectFormat::Sha1 => "sha1",
            ObjectFormat::Sha256 => "sha256",
        };
        let start = Instant::now();
        let run = |arguments: &[OsString], input: Vec<u8>| {
            cancel.check()?;
            let mut runner: Runner = self.runner.clone();
            runner.limits.timeout = runner
                .limits
                .timeout
                .min(CHECK_BUDGET.saturating_sub(start.elapsed()));
            runner.prediction_sandbox(scratch, arguments, input, cancel, &empty)
        };
        run(
            &args(&[
                "init",
                "--bare",
                "--template=",
                &format!("--object-format={format}"),
                ".",
            ]),
            Vec::new(),
        )?
        .success("commit-prediction")?;
        let objects = runner::git_path(&self.info().common_dir.join("objects"));
        let objects = objects
            .to_str()
            .ok_or_else(|| {
                Error::Unsupported("Prediction needs a UTF-8 object directory path.".into())
            })?
            .replace('\\', "/");
        if objects.contains(['\n', '\r']) {
            return Err(Error::Unsupported(
                "Prediction object directory contains a line break.".into(),
            ));
        }
        fs::write(
            scratch.join("objects/info/alternates"),
            format!("{objects}\n"),
        )
        .map_err(|e| Error::io("connect read-only prediction objects", e))?;
        // Detached HEAD permits ordinary attribute fallback, but no external attributes
        // are loaded. Guarded merge/filter attributes are refused before simulation.
        fs::write(scratch.join("HEAD"), format!("{head}\n"))
            .map_err(|e| Error::io("set isolated prediction base", e))?;
        let mut config = args(&["--no-replace-objects", "-c"]);
        config.push(
            format!(
                "core.attributesFile={}",
                runner::git_path(&empty).to_string_lossy()
            )
            .into(),
        );
        for (key, value) in settings(&guard.config)? {
            if key != "core.attributesfile" {
                config.push("-c".into());
                config.push(format!("{key}={value}").into());
            }
        }
        let mut current = parse::oid(
            parse::line(
                &run(
                    &args(&["rev-parse", &format!("{head}^{{tree}}")]),
                    Vec::new(),
                )?
                .success("commit-prediction")?,
            ),
            "commit-prediction",
        )?;
        let empty_tree = parse::oid(
            parse::line(
                &run(
                    &args(&["hash-object", "-w", "-t", "tree", "--stdin"]),
                    Vec::new(),
                )?
                .success("commit-prediction")?,
            ),
            "commit-prediction",
        )?;
        let mut paths: BTreeSet<Vec<u8>> = guard.paths.iter().cloned().collect();
        for (i, commit) in commits.iter().enumerate() {
            cancel.check()?;
            if start.elapsed() >= CHECK_BUDGET {
                let mut p = CommitPrediction::unavailable(commits.len(), PredictionReason::Limit);
                p.checked = i;
                return Ok((p, paths.into_iter().collect()));
            }
            let mainline = match kind {
                ReplayKind::Commit(r) => r.mainline.unwrap_or(1),
                ReplayKind::Rebase { .. } => 1,
            };
            let parent = commit.parents.get(mainline - 1).unwrap_or(&empty_tree);
            let (base, incoming) = if matches!(kind, ReplayKind::Commit(r) if r.operation == CommitOperation::Revert)
            {
                (&commit.id, parent)
            } else {
                (parent, &commit.id)
            };
            let mut a = config.clone();
            a.extend(args(&[
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                "-z",
                &format!("--merge-base={base}"),
                current.as_str(),
                incoming.as_str(),
            ]));
            let out = run(&a, Vec::new())?;
            let code = out.status.code();
            if !matches!(code, Some(0 | 1)) {
                out.success("commit-prediction")?;
                unreachable!();
            }
            let (tree, conflicts) = merge_output(&out.stdout)?;
            let tree_bytes = run(
                &args(&["ls-tree", "-r", "-z", "--full-tree", tree.as_str()]),
                Vec::new(),
            )?
            .success("commit-prediction")?;
            let (tree_paths, submodule) = tree_paths(&tree_bytes)?;
            paths.extend(tree_paths);
            paths.extend(conflicts.iter().map(|p| p.as_bytes().to_vec()));
            if paths.len() > PATH_LIMIT || submodule {
                let mut p = CommitPrediction::unavailable(
                    commits.len(),
                    if submodule {
                        PredictionReason::Submodules
                    } else {
                        PredictionReason::Limit
                    },
                );
                p.checked = i;
                return Ok((p, paths.into_iter().collect()));
            }
            // Rebase uses --keep-empty: an originally empty commit is kept, while
            // a nonempty commit that becomes empty pauses under --empty=stop.
            if code == Some(0) && tree == current && matches!(kind, ReplayKind::Rebase { .. }) {
                let parent_tree = if commit.parents.is_empty() {
                    empty_tree.clone()
                } else {
                    parse::oid(
                        parse::line(
                            &run(
                                &args(&["rev-parse", &format!("{parent}^{{tree}}")]),
                                Vec::new(),
                            )?
                            .success("rebase-prediction")?,
                        ),
                        "rebase-prediction",
                    )?
                };
                if parent_tree == commit.tree {
                    continue;
                }
            }
            if code == Some(1) || tree == current {
                return Ok((
                    CommitPrediction {
                        kind: if code == Some(1) {
                            PredictionKind::Conflicts
                        } else {
                            PredictionKind::Empty
                        },
                        checked: i + 1,
                        total: commits.len(),
                        stopped_at: Some(i),
                        paths: conflicts,
                        reason: None,
                    },
                    paths.into_iter().collect(),
                ));
            }
            current = tree;
        }
        Ok((
            CommitPrediction {
                kind: PredictionKind::Clean,
                checked: commits.len(),
                total: commits.len(),
                stopped_at: None,
                paths: Vec::new(),
                reason: None,
            },
            paths.into_iter().collect(),
        ))
    }
}

pub(crate) fn is_attributes(path: &[u8]) -> bool {
    path.rsplit(|b| *b == b'/').next() == Some(b".gitattributes")
}

fn settings(bytes: &[u8]) -> Result<Vec<(&str, &str)>> {
    let mut values = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let s = std::str::from_utf8(record)
            .map_err(|_| Error::parse("commit-prediction", "non-UTF-8 merge setting"))?;
        let (k, v) = s
            .split_once('\n')
            .ok_or_else(|| Error::parse("commit-prediction", "invalid merge setting"))?;
        values.push((k, v));
    }
    Ok(values)
}
fn unsupported(guard: &PredictionGuard) -> Result<Option<PredictionReason>> {
    if !guard.replacements.is_empty() || guard.grafts {
        return Ok(Some(PredictionReason::ReplacedObjects));
    }
    for (k, v) in settings(&guard.config)? {
        match k {
            "merge.renames"
            | "merge.renamelimit"
            | "merge.directoryrenames"
            | "merge.conflictstyle"
            | "diff.renames"
            | "diff.renamelimit"
            | "core.attributesfile" => {}
            "merge.renormalize"
                if matches!(
                    v.to_ascii_lowercase().as_str(),
                    "false" | "no" | "off" | "0"
                ) => {}
            _ => return Ok(Some(PredictionReason::MergeSettings)),
        }
    }
    let fields = guard.attributes.split(|b| *b == 0).collect::<Vec<_>>();
    if fields.last() != Some(&b"".as_slice()) || (fields.len() - 1) % 3 != 0 {
        return Err(Error::parse(
            "commit-prediction",
            "invalid attribute response",
        ));
    }
    for row in fields[..fields.len() - 1].as_chunks::<3>().0 {
        if row[2] != b"unspecified" && !(row[1] != b"merge" && row[2] == b"unset") {
            return Ok(Some(PredictionReason::Attributes));
        }
    }
    Ok(None)
}
fn tree_paths(bytes: &[u8]) -> Result<(Vec<Vec<u8>>, bool)> {
    let mut paths = Vec::new();
    let mut submodule = false;
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let at = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| Error::parse("commit-prediction", "invalid tree entry"))?;
        submodule |= record.starts_with(b"160000 ");
        paths.push(
            RepoPath::new(record[at + 1..].to_vec())?
                .as_bytes()
                .to_vec(),
        );
    }
    Ok((paths, submodule))
}
fn merge_output(bytes: &[u8]) -> Result<(ObjectId, Vec<RepoPath>)> {
    let mut fields = bytes.split(|b| *b == 0);
    let tree = parse::oid(fields.next().unwrap_or_default(), "commit-prediction")?;
    let mut paths = Vec::new();
    for path in fields.filter(|p| !p.is_empty()) {
        paths.push(RepoPath::new(path.to_vec())?);
    }
    Ok((tree, paths))
}
