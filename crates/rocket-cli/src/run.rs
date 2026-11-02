use crate::{
    args::{Command, Options},
    output::{self as out, Outcome, Problem},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::{ffi::OsStr, time::Duration};

fn argument(value: &OsStr) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value
            .to_str()
            .map(|s| s.as_bytes().to_vec())
            .ok_or_else(|| {
                Error::InvalidInput(
                    "Git name/path arguments must be valid Unicode on Windows".into(),
                )
            })
    }
}
pub(crate) fn repo_path(value: &OsStr) -> Result<RepoPath> {
    let bytes = argument(value)?;
    #[cfg(windows)]
    let bytes = bytes
        .into_iter()
        .map(|b| if b == b'\\' { b'/' } else { b })
        .collect::<Vec<_>>();
    RepoPath::new(bytes)
}

pub fn execute(
    options: &Options,
    log: Option<&DiagnosticLog>,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    cancel.check()?;
    let limits = CommandLimits {
        timeout: Duration::from_secs(options.timeout_secs),
        ..Default::default()
    };
    let client =
        GitClient::discover_with_options(options.git.as_deref(), limits, log.cloned(), cancel)?;
    match &options.command {
        Command::Init {
            destination,
            branch,
        } => return Ok(init(client.init_repository(destination, branch, cancel)?)),
        Command::Clone {
            source,
            destination,
        } => {
            return Ok(clone_report(client.clone_repository(
                source,
                destination,
                None,
                cancel,
            )?));
        }
        _ => (),
    }
    let repo = client.open(&options.repo, cancel)?;
    let result = match &options.command {
        Command::Info => Outcome::success(out::info(repo.info())),
        Command::Status => Outcome::success(out::status(&repo.status(cancel)?)),
        Command::Operation => Outcome::success(out::operation(&repo.operation_state(cancel)?)),
        Command::History {
            all,
            revision,
            limit,
            order,
        } => {
            let start = if *all {
                HistoryStart::AllBranches
            } else if let Some(revision) = revision {
                HistoryStart::Revision(revision.clone())
            } else {
                HistoryStart::Head
            };
            Outcome::success(out::history(&repo.history_ordered(start, *limit, *order, cancel)?))
        }
        Command::CommitUrl { revision, remote, config } => {
            let id = repo.resolve_commit(revision, cancel)?;
            let links = repo.commit_browser_links(&id, remote.as_ref().map(|r| r.as_bytes()), config.clone(), cancel)?;
            if links.len() != 1 {
                return Err(Error::InvalidInput(if links.is_empty() {
                    "No supported browser URL for this remote. For a custom host, provide --remote, --provider and --server."
                } else { "Several browser destinations match. Choose one with --remote (and --provider/--server if needed)." }.into()));
            }
            let link = &links[0];
            Outcome::success(serde_json::json!({"commit":id.as_str(), "remote":out::bytes(&link.remote),
                "provider":link.provider.key(),"url":link.url,"remote_presence":"not_checked"}))
        }
        Command::Review { revision, path, blame } => {
            let id = repo.resolve_commit(revision, cancel)?;
            let path = path.as_ref().map(|p| crate::review::path(p)).transpose()?;
            if let Some(path) = &path { repo.historical_file(&id, path, cancel)?; }
            cancel.check()?;
            let pid = crate::review::launch(repo.info(), &id, path.as_ref(), *blame)?;
            Outcome::success(serde_json::json!({"launched":true,"pid":pid,"commit":id.as_str(),
                "path":path.as_ref().map(|p|out::bytes(p.as_bytes())), "mode":if *blame {"blame"} else if path.is_some() {"file"} else {"commit"},
                "note":"The native window loads asynchronously; check its status for read errors."}))
        }
        Command::Diff {
            staged,
            base,
            target,
            context,
            paths,
        } => {
            let target = match (base, target) {
                (Some(base), Some(target)) => DiffTarget::Between {
                    base: base.clone(),
                    target: target.clone(),
                },
                _ if *staged => DiffTarget::Staged,
                _ => DiffTarget::Unstaged,
            };
            let options = DiffOptions {
                target,
                context_lines: *context,
                paths: paths.iter().map(|p| repo_path(p)).collect::<Result<_>>()?,
            };
            let patch = repo.diff(&options, cancel)?.patch;
            Outcome::success(json!({"patch":out::bytes(&patch),"byte_count":patch.len()}))
        }
        Command::Stage {
            all,
            paths,
            unstage,
        } => {
            let paths = paths
                .iter()
                .map(|p| repo_path(p))
                .collect::<Result<Vec<_>>>()?;
            let report = match (*unstage, *all) {
                (false, false) => repo.stage(&paths, cancel)?,
                (false, true) => repo.stage_all(cancel)?,
                (true, false) => repo.unstage(&paths, cancel)?,
                (true, true) => repo.unstage_all(cancel)?,
            };
            write(report)
        }
        Command::Commit(message) => crate::commits::commit(repo.commit_with_undo(message, cancel)?),
        Command::BranchList => Outcome::success(out::branches(&repo.branches(cancel)?)),
        Command::BranchCreate { name, start } => {
            branch(repo.create_branch(argument(name)?, start.as_deref(), cancel)?)
        }
        Command::BranchSwitch(name) => branch(repo.switch_branch(argument(name)?, cancel)?),
        Command::BranchRename(old, new) => {
            branch(repo.rename_branch(argument(old)?, argument(new)?, cancel)?)
        }
        Command::BranchDelete(name) => branch(repo.delete_branch(argument(name)?, cancel)?),
        Command::StashList => Outcome::success(out::stashes(&repo.stashes(cancel)?)),
        Command::StashSave { message, untracked, paths } if paths.is_empty() => {
            stash(repo.push_stash(message.as_deref(), *untracked, cancel)?)
        }
        // Only these files' changes are saved; other edits stay in the working tree.
        Command::StashSave { message, untracked, paths } => {
            let paths = paths.iter().map(|p| repo_path(p)).collect::<Result<Vec<_>>>()?;
            stash(repo.push_stash_paths(message.as_deref(), *untracked, &paths, cancel)?)
        }
        Command::StashApply { id, index } => {
            stash(repo.apply_stash(&ObjectId::parse(id)?, *index, cancel)?)
        }
        Command::StashDrop { index, expected } => {
            let id = ObjectId::parse(expected)?;
            let entries = repo.stashes(cancel)?;
            let selected = entries
                .iter()
                .find(|s| s.index == *index && s.id == id)
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "Stash selection changed; list stashes again and use the expected full ID."
                            .into(),
                    )
                })?;
            stash(repo.drop_stash(selected, cancel)?)
        }
        Command::MergeStart { revision, mode } => {
            let mut out=merge(repo.merge(revision, *mode, cancel)?);
            out.data["mode"]=json!(match mode{MergeMode::FastForwardOnly=>"fast_forward_only",MergeMode::AllowFastForward=>"allow_fast_forward",MergeMode::NoFastForward=>"no_fast_forward",MergeMode::Squash=>"squash",MergeMode::NoCommit=>"no_commit"});
            if *mode==MergeMode::Squash {out.data["completion"]=json!("Inspect Changes, resolve any conflicts, then create an ordinary commit. Squash has no merge continuation/abort state; use reviewed discard/restore to undo prepared files.");}
            if *mode==MergeMode::NoCommit {out.data["completion"]=json!("Inspect the reported operation state. A prepared merge can be continued or aborted; HEAD has not advanced unless the observed state says otherwise.");}
            out
        },
        Command::MergeContinue(message) => merge(repo.continue_merge(message, cancel)?),
        Command::MergeAbort => merge(repo.abort_merge(cancel)?),
        Command::RemoteList => Outcome::success(out::remotes(&repo.remotes(cancel)?)),
        Command::RemoteAdd(name, url) => {
            remote(repo.add_remote(&argument(name)?, &argument(url)?, cancel)?)
        }
        Command::RemoteSetUrl { name, url, push } => {
            let name = argument(name)?;
            let remotes = repo.remotes(cancel)?;
            let expected = remotes
                .iter()
                .find(|r| r.name == name)
                .ok_or_else(|| Error::InvalidInput("Remote not found; use remote list.".into()))?;
            remote(repo.set_remote_url(
                expected,
                if *push {
                    RemoteUrlKind::Push
                } else {
                    RemoteUrlKind::Fetch
                },
                Some(&argument(url)?),
                cancel,
            )?)
        }
        Command::RemoteRemove(name) => {
            let name = argument(name)?;
            let remotes = repo.remotes(cancel)?;
            let expected = remotes
                .iter()
                .find(|r| r.name == name)
                .ok_or_else(|| Error::InvalidInput("Remote not found; use remote list.".into()))?;
            remote(repo.remove_remote(expected, cancel)?)
        }
        Command::Fetch { remote: name, tags: false } => fetch(repo.fetch(argument(name)?, None, cancel)?),
        // All remote tags; a different local tag of the same name is never replaced.
        Command::Fetch { remote: name, tags: true } => {
            fetch(repo.fetch_with_tags(argument(name)?, FetchTags::All, None, cancel)?)
        }
        Command::Pull {
            strategy: PullStrategy::FastForward,
            preview: false,
        } => {
            let mut result = pull(repo.pull_ff_only(None, cancel)?);
            result.data["strategy"] = json!(PullStrategy::FastForward.code());
            result
        }
        Command::Pull { strategy, preview } => pull_strategy(&repo, *strategy, *preview, cancel)?,
        Command::RebaseRecovery { action, preview } => {
            rebase_recovery(&repo, *action, *preview, cancel)?
        }
        Command::Push {
            remote,
            local,
            destination,
            upstream,
        } => push(repo.push(
            argument(remote)?,
            argument(local)?,
            argument(destination)?,
            PushOptions {
                set_upstream: *upstream,
            },
            cancel,
        )?),
        Command::PushLease { .. } | Command::PushAtomic { .. }
        | Command::TagList { .. } | Command::TagInspect(_) | Command::TagCreate { .. }
        | Command::TagDelete { .. } | Command::TagRemote(_) | Command::TagPublish { .. }
        | Command::TagDeleteRemote { .. } | Command::WorktreeList | Command::WorktreeCreate { .. }
        | Command::WorktreeRemove { .. } => crate::advanced::execute(&options.command, &repo, cancel)?,
        Command::Hunks { .. } | Command::StageLines { .. } | Command::StashShow { .. }
        | Command::StashApplySelection { .. } | Command::StashSaveLines { .. }
        | Command::StashPop { .. } => crate::selection::execute(&options.command, &repo, cancel)?,
        Command::ConflictShow(_) | Command::ConflictResolve { .. } | Command::ConflictKeep { .. }
        | Command::ConflictMarkResolved(_) => crate::conflicts::execute(&options.command, &repo, cancel)?,
        Command::BisectStatus | Command::Bisect { .. } | Command::BisectRun { .. } => {
            crate::bisect::execute(&options.command, &repo, cancel)?
        }
        Command::UndoCommit { .. } | Command::CommitAction { .. } | Command::RebaseStart { .. }
        | Command::Reset { .. } => crate::commits::execute(&options.command, &repo, cancel)?,
        Command::Blame { .. } | Command::FileHistory { .. } | Command::Search { .. }
        | Command::Compare { .. } | Command::Reflog { .. } | Command::Verify { .. }
        | Command::SigningShow => crate::reads::execute(&options.command, &repo, cancel)?,
        Command::RemoteRename { .. } | Command::RemotePrune { .. } | Command::BranchDeleteRemote { .. }
        | Command::BranchFastForward { .. } | Command::WorktreeMaintain { .. } | Command::IdentityShow
        | Command::IdentitySet { .. } => crate::maintenance::execute(&options.command, &repo, cancel)?,
        Command::LfsStatus | Command::LfsChange { .. } | Command::LfsCheckout { .. }
        | Command::LfsLocks(_) | Command::LfsLock { .. } => crate::lfs::execute(&options.command, &repo, cancel)?,
        Command::SshKeys | Command::SshCheck(_) | Command::SshBinding { .. } => {
            crate::ssh::execute(&options.command, &repo, cancel)?
        }
        Command::Help | Command::Version | Command::Init { .. } | Command::Clone { .. } => {
            unreachable!("handled before opening repository")
        }
    };
    Ok(result)
}

pub(crate) fn write(report: WriteReport) -> Outcome {
    let mut o = Outcome::success(
        json!({"command_result":out::observation(&report.result,out::write_output)}),
    );
    o.primary(&report.result);
    let status = o.observe("status", &report.status, out::status);
    o.data["status"] = status;
    o
}
fn branch(report: BranchWriteReport) -> Outcome {
    let mut o = write(report.write);
    let branches = o.observe("branches", &report.branches, |b| out::branches(b));
    o.data["branches"] = branches;
    o
}
fn stash(report: StashWriteReport) -> Outcome {
    let mut o = write(report.write);
    let stashes = o.observe("stashes", &report.stashes, |s| out::stashes(s));
    o.data["stashes"] = stashes;
    o
}
fn merge(report: MergeWriteReport) -> Outcome {
    let mut o = write(report.write);
    let branches = o.observe("branches", &report.branches, |b| out::branches(b));
    o.data["branches"] = branches;
    let operation = o.observe("operation", &report.state, out::operation);
    o.data["operation"] = operation;
    if let Some(failure) = report.failure {
        o.data["merge_failure"] = json!({"code":failure.code(),"guidance":failure.guidance()});
        if o.error
            .as_ref()
            .is_some_and(|e| e.exit != 130 && e.exit != 124)
        {
            o.error = Some(Problem::new(
                failure.code(),
                "Merge needs attention.",
                failure.guidance(),
                1,
            ));
        }
    }
    o
}
pub(crate) fn remote(report: RemoteWriteReport) -> Outcome {
    let mut o = Outcome::success(
        json!({"command_result":out::observation(&report.result,out::write_output)}),
    );
    o.primary(&report.result);
    let remotes = o.observe("remotes", &report.remotes, |r| out::remotes(r));
    o.data["remotes"] = remotes;
    let branches = o.observe("branches", &report.branches, |b| out::branches(b));
    o.data["branches"] = branches;
    o.data["status"] = report
        .status
        .as_ref()
        .map_or(Value::Null, |r| o.observe("status", r, out::status));
    o
}
fn fetch_updates(updates: &[FetchUpdate]) -> Value {
    json!(updates.iter().map(|u|json!({"kind":match u.kind {FetchUpdateKind::FastForward=>"fast_forward",FetchUpdateKind::Forced=>"forced",FetchUpdateKind::New=>"new",FetchUpdateKind::UpToDate=>"up_to_date"},"old":u.old.as_ref().map(ObjectId::as_str),"new":u.new.as_str(),"reference":out::bytes(&u.reference)})).collect::<Vec<_>>())
}
fn fetched(report: &Result<FetchOutput>) -> Value {
    out::observation(
        report,
        |f| json!({"command":out::write_output(&f.output),"updates":out::observation(&f.updates,|u|fetch_updates(u))}),
    )
}
fn fetch(report: FetchReport) -> Outcome {
    let mut o = Outcome::success(json!({"command_result":fetched(&report.result)}));
    o.primary(&report.result);
    if let Ok(f) = &report.result {
        o.observe("fetch_updates", &f.updates, |u| fetch_updates(u));
    }
    o.data["branches"] = o.observe("branches", &report.branches, |b| out::branches(b));
    o.data["status"] = report
        .status
        .as_ref()
        .map_or(Value::Null, |s| o.observe("status", s, out::status));
    o
}
fn pull(report: PullReport) -> Outcome {
    let mut o = Outcome::success(
        json!({"fetch":fetched(&report.fetch),"upstream":{"branch":out::bytes(&report.upstream.branch),"remote":out::bytes(&report.upstream.remote),"remote_reference":out::bytes(&report.upstream.remote_reference),"tracking_reference":out::bytes(&report.upstream.tracking_reference)},
        "update":out::optional(&report.update,|u|json!({"kind":match u.kind {PullUpdateKind::FastForward=>"fast_forward",PullUpdateKind::UpToDate=>"up_to_date"},"before":u.before.as_str(),"fetched":u.fetched.as_str(),"command":out::write_output(&u.output),"verification":out::observation(&u.verification,|_|Value::Null),"error_diagnostics":u.error_diagnostics}))}),
    );
    o.primary(&report.fetch);
    if let Ok(f) = &report.fetch {
        if let Err(e) = &f.updates {
            o.error = Some(Problem::backend(e));
        }
        if let Some(update) = &report.update {
            o.primary(update);
            if let Ok(u) = update {
                o.observe("branch_verification", &u.verification, |_| Value::Null);
                o.advisory(u.error_diagnostics);
            }
        } else if o.error.is_none() {
            o.error = Some(Problem::new(
                "update_not_attempted",
                "Fetch succeeded but the local branch was not updated.",
                "Inspect status and upstream before retrying.",
                1,
            ));
        }
    }
    o.data["status"] = o.observe("status", &report.status, out::status);
    o.data["branches"] = o.observe("branches", &report.branches, |b| out::branches(b));
    o.data["operation"] = o.observe("operation", &report.state, out::operation);
    o
}
fn push_update(update: &PushUpdate) -> Value {
    let (kind, rejection) = match update.kind {
        PushUpdateKind::New => ("new", None),
        PushUpdateKind::FastForward => ("fast_forward", None),
        PushUpdateKind::UpToDate => ("up_to_date", None),
        PushUpdateKind::Rejected(r) => (
            "rejected",
            Some(match r {
                PushRejection::StaleLease => "stale_lease",
                PushRejection::NonFastForward => "non_fast_forward",
                PushRejection::FetchFirst => "fetch_first",
                PushRejection::RemoteRejected => "remote_rejected",
                PushRejection::RemoteFailure => "remote_failure",
                PushRejection::Other => "other",
            }),
        ),
    };
    json!({"kind":kind,"rejection":rejection,"local_reference":out::bytes(&update.local_reference),"remote_reference":out::bytes(&update.remote_reference)})
}
fn push(report: PushReport) -> Outcome {
    let mut o = Outcome::success(
        json!({"command_result":out::observation(&report.result,out::write_output),"remote_outcome":report.remote_outcome.code(),"update":out::optional(&report.update,push_update),"upstream":out::optional(&report.upstream,|_|Value::Null),"error_diagnostics":report.error_diagnostics}),
    );
    o.primary(&report.result);
    if let Some(u) = &report.update {
        o.observe("push_update", u, push_update);
    }
    if let Some(u) = &report.upstream {
        o.observe("upstream", u, |_| Value::Null);
    }
    if report.remote_outcome == PushRemoteOutcome::Unknown {
        o.warnings.push(json!({"code":"remote_outcome_unknown","message":"The remote may already have changed. Check it before retrying; local status cannot establish the remote outcome."}));
    }
    o.advisory(report.error_diagnostics);
    o.data["status"] = report
        .status
        .as_ref()
        .map_or(Value::Null, |s| o.observe("status", s, out::status));
    o.data["branches"] = o.observe("branches", &report.branches, |b| out::branches(b));
    o
}
fn clone_report(report: CloneReport) -> Outcome {
    let published = matches!(report.publication, Some(Ok(())));
    let mut o = Outcome::success(
        json!({"destination":out::os_path(&report.destination),"staging_directory":out::os_path(&report.staging_directory),"published":published,"command_result":out::observation(&report.command,out::write_output),"verification":out::optional(&report.verification,|_|Value::Null),"publication":out::optional(&report.publication,|_|Value::Null),"error_diagnostics":report.error_diagnostics}),
    );
    if !published {
        o.primary(&report.command);
        if o.error.is_none()
            && let Some(r) = &report.verification
        {
            o.primary(r);
        }
        if o.error.is_none()
            && let Some(r) = &report.publication
        {
            o.primary(r);
        }
        if o.error.is_none() {
            o.error = Some(Problem::new(
                "not_published",
                "Clone was not published.",
                "Inspect the retained staging directory. Existing destination files were not replaced.",
                1,
            ));
        }
    }
    o.data["repository"] = report.repository.as_ref().map_or(Value::Null, |r| {
        o.observe("repository", r, |r| out::info(r.info()))
    });
    o.data["cleanup"] = report
        .cleanup
        .as_ref()
        .map_or(Value::Null, |r| o.observe("cleanup", r, |_| Value::Null));
    o.advisory(report.error_diagnostics);
    o
}
fn init(report: InitReport) -> Outcome {
    let published = matches!(report.publication, Some(Ok(())));
    let mut o = Outcome::success(
        json!({"destination":out::os_path(&report.destination),"staging_directory":out::os_path(&report.staging_directory),"published":published,"preparation":out::observation(&report.preparation,|_|Value::Null),"publication":out::optional(&report.publication,|_|Value::Null)}),
    );
    if !published {
        o.primary(&report.preparation);
        if o.error.is_none()
            && let Some(r) = &report.publication
        {
            o.primary(r);
        }
        if o.error.is_none() {
            o.error = Some(Problem::new(
                "not_published",
                "Repository creation was not published.",
                "Inspect the retained staging directory. Existing files were not replaced.",
                1,
            ));
        }
    }
    o.data["repository"] = report.repository.as_ref().map_or(Value::Null, |r| {
        o.observe("repository", r, |r| out::info(r.info()))
    });
    o.data["cleanup"] = report
        .cleanup
        .as_ref()
        .map_or(Value::Null, |r| o.observe("cleanup", r, |_| Value::Null));
    o
}

// Keep the original fast-forward JSON shape above for existing scripts. Explicit
// strategies use the same fetched ticket and revalidated integration as the UI.
fn pull_strategy(
    repo: &Repository,
    strategy: PullStrategy,
    preview_only: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let report = repo.fetch_for_pull(None, cancel)?;
    let mut o = Outcome::success(
        json!({"strategy":strategy.code(),"preview_only":preview_only,
        "fetch":fetched(&report.fetch),"preview":null,"integration":null}),
    );
    o.primary(&report.fetch);
    if let Ok(fetch) = &report.fetch {
        o.primary(&fetch.updates);
    }
    o.data["status"] = o.observe("status", &report.status, out::status);
    o.data["branches"] = o.observe("branches", &report.branches, |v| out::branches(v));
    o.data["operation"] = o.observe("operation", &report.state, out::operation);
    let fetched = match report.prepared {
        Some(Ok(fetched)) if o.error.is_none() => fetched,
        Some(Err(error)) => {
            o.error = Some(Problem::backend(&error));
            return Ok(o);
        }
        _ => {
            if o.error.is_none() {
                o.error = Some(Problem::new(
                    "pull_not_prepared",
                    "Fetch did not produce an integration review.",
                    "Inspect the fetch report and repository state before retrying.",
                    1,
                ));
            }
            return Ok(o);
        }
    };
    let preview = match repo.preview_pull(&fetched, strategy, cancel) {
        Ok(preview) => preview,
        Err(error) => {
            o.error = Some(Problem::backend(&error));
            refresh_pull(repo, &mut o);
            return Ok(o);
        }
    };
    o.data["preview"] = json!({"original":fetched.original().as_str(),"target":fetched.target().as_str(),
        "upstream":{"branch":out::bytes(&fetched.upstream().branch),"remote":out::bytes(&fetched.upstream().remote),
        "tracking_reference":out::bytes(&fetched.upstream().tracking_reference)},
        "incoming":preview.incoming(),"local_only":preview.current_only(),"can_apply":preview.can_apply(),
        "replay":preview.replay().iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),
        "recovery_reference":preview.recovery_reference(),
        "prediction":preview.prediction().map(out::prediction)});
    if preview_only {
        return Ok(o);
    }
    let applied = match repo.apply_pull(&preview, cancel) {
        Ok(PullApplyReport::Merge(report)) => merge(*report),
        Ok(PullApplyReport::Rebase(report)) => rebase(*report),
        Err(error) => {
            o.error = Some(Problem::backend(&error));
            refresh_pull(repo, &mut o);
            return Ok(o);
        }
    };
    for key in ["status", "branches", "operation"] {
        o.data[key] = applied.data[key].clone();
    }
    o.data["integration"] = applied.data;
    o.error = applied.error;
    o.warnings.extend(applied.warnings);
    Ok(o)
}
fn refresh_pull(repo: &Repository, o: &mut Outcome) {
    // A cancelled request still needs current observations, with fresh tokens.
    o.data["status"] = o.observe(
        "status",
        &repo.status(&CancellationToken::new()),
        out::status,
    );
    o.data["branches"] = o.observe("branches", &repo.branches(&CancellationToken::new()), |v| {
        out::branches(v)
    });
    o.data["operation"] = o.observe(
        "operation",
        &repo.operation_state(&CancellationToken::new()),
        out::operation,
    );
}
fn rebase_recovery(
    repo: &Repository,
    action: RebaseAction,
    preview_only: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let preview = repo.preview_rebase(action, "", cancel)?;
    if preview.runs_commands() {
        return Err(Error::Unsupported("This CLI recovery flow does not run interactive rebase commands. Finish that reviewed plan in Rocket's desktop UI.".into()));
    }
    let value = json!({"action":action.label(),"branch":out::bytes(preview.branch()),
        "original":preview.original().as_str(),"onto":preview.onto().map(ObjectId::as_str),
        "stopped":preview.stopped().map(ObjectId::as_str),"recovery_reference":preview.recovery_reference(),
        "discards_tracked_changes":matches!(action, RebaseAction::Skip | RebaseAction::Abort)});
    let mut o = if preview_only {
        Outcome::success(json!({"preview_only":true}))
    } else {
        match repo.apply_rebase(&preview, cancel) {
            Ok(r) => rebase(r),
            Err(e) => {
                let mut o = Outcome::success(json!({}));
                o.error = Some(Problem::backend(&e));
                refresh_pull(repo, &mut o);
                o
            }
        }
    };
    o.data["preview"] = value;
    Ok(o)
}
pub(crate) fn rebase(report: RebaseWriteReport) -> Outcome {
    let mut o = write(report.write);
    o.data["branches"] = o.observe("branches", &report.branches, |v| out::branches(v));
    o.data["operation"] = o.observe("operation", &report.state, out::operation);
    o.data["recovery_reference"] = json!(report.recovery_reference);
    // A stop the reviewed plan asked for (edit or stop_after), not a conflict.
    o.data["stop"] = match &report.stop {
        None => Value::Null,
        Some(RebaseStop::Edit(id)) => json!({"kind":"edit","commit":id.as_str()}),
        Some(RebaseStop::Break) => json!({"kind":"break","commit":null}),
        Some(RebaseStop::Command(_)) => json!({"kind":"command","commit":null}),
    };
    o.data["journal_cleanup"] =
        o.observe("journal_cleanup", &report.journal_cleanup, |_| Value::Null);
    if let Some(failure) = report.failure {
        o.data["rebase_failure"] = json!({"code":failure.code(),"guidance":failure.guidance()});
        if o.error
            .as_ref()
            .is_none_or(|e| e.exit != 130 && e.exit != 124)
        {
            o.error = Some(Problem::new(
                failure.code(),
                "Rebase needs attention.",
                failure.guidance(),
                1,
            ));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_git_discovery_honors_cancellation_before_resolving_paths() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        for executable in [None, Some(std::path::Path::new("missing-git-executable"))] {
            assert!(matches!(
                GitClient::discover_with_options(
                    executable,
                    CommandLimits::default(),
                    None,
                    &cancel
                ),
                Err(Error::Cancelled)
            ));
        }
    }
}
