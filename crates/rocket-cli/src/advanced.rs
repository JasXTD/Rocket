//! CLI adapters for existing reviewed backend operations. Confirmation values
//! pin objects across invocations; the backend rechecks config and refs again
//! under repository write ownership immediately before each mutation.
use crate::{
    args::Command,
    output::{self as out, Outcome},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct AtomicSelection {
    pub local: String,
    pub destination: String,
    pub source: Option<String>,
    pub expected: Option<String>,
}

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::PushLease {
            remote,
            local,
            destination,
            source,
            expected,
            preview,
        } => {
            let p = repo.preview_force_push(
                remote.as_bytes(),
                local.as_bytes(),
                destination.as_bytes(),
                cancel,
            )?;
            let review = json!({"remote":out::bytes(p.remote()),"local_branch":out::bytes(p.local_branch()),
                "remote_branch":out::bytes(p.remote_branch()),"source":p.source().as_str(),"expected":p.expected().as_str(),
                "added_count":p.added_count(),"removed_count":p.removed_count(),"can_apply":p.can_apply(),
                "added":p.added().iter().map(commit).collect::<Vec<_>>(),"removed":p.removed().iter().map(commit).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            if source.as_deref() != Some(p.source().as_str())
                || expected.as_deref() != Some(p.expected().as_str())
            {
                return Err(Error::ForcePush(ForcePushFailure::StalePreview));
            }
            let r = repo.apply_force_push(&p, cancel)?;
            let mut result = Outcome::success(
                json!({"preview":review,"command_result":out::observation(&r.result,out::write_output),
                "remote_outcome":r.remote_state.code(),"observed":out::observation(&r.observed,oid_option),
                "verification":out::observation(&r.verification,|_|Value::Null),"error_diagnostics":r.error_diagnostics}),
            );
            finish(&mut result, &r.result, &r.verification);
            result.advisory(r.error_diagnostics);
            uncertain(&mut result, r.remote_state == ForcePushRemoteState::Unknown);
            Ok(result)
        }
        Command::PushAtomic {
            remote,
            branches,
            preview,
        } => {
            let selection = branches
                .iter()
                .map(|b| AtomicPushSelection {
                    local_branch: b.local.as_bytes().to_vec(),
                    remote_branch: b.destination.as_bytes().to_vec(),
                })
                .collect::<Vec<_>>();
            let p = repo.preview_atomic_push(remote.as_bytes(), &selection, cancel)?;
            let review = json!({"remote":out::bytes(p.remote()),"atomic":true,"can_apply":p.can_apply(),"branches":p.entries().iter().map(|e|json!({
                "local_branch":out::bytes(&e.selection.local_branch),"remote_branch":out::bytes(&e.selection.remote_branch),
                "source":e.source.as_str(),"expected":e.expected.as_ref().map(ObjectId::as_str),"kind":e.kind.code()})).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            if p.entries().len() != branches.len()
                || p.entries().iter().zip(branches).any(|(e, b)| {
                    b.source.as_deref() != Some(e.source.as_str())
                        || b.expected.as_deref() != e.expected.as_ref().map(ObjectId::as_str)
                })
            {
                return Err(Error::AtomicPush(AtomicPushFailure::StalePreview));
            }
            let r = repo.apply_atomic_push(&p, cancel)?;
            let mut result = Outcome::success(
                json!({"preview":review,"command_result":out::observation(&r.result,out::write_output),
                "remote_outcome":r.remote_state.code(),"observed":out::observation(&r.observed,|ids|json!(ids.iter().map(oid_option).collect::<Vec<_>>())),
                "verification":out::observation(&r.verification,|_|Value::Null),"error_diagnostics":r.error_diagnostics}),
            );
            finish(&mut result, &r.result, &r.verification);
            result.advisory(r.error_diagnostics);
            uncertain(
                &mut result,
                r.remote_state == AtomicPushRemoteState::Unknown,
            );
            Ok(result)
        }
        Command::TagList { limit, order } => {
            let page = repo.tags_ordered(*limit, *order, cancel)?;
            Ok(Outcome::success(
                json!({"entries":page.entries.iter().map(tag).collect::<Vec<_>>(),"has_more":page.more,
                "order":if *order == TagOrder::Newest {"newest"} else {"name"}}),
            ))
        }
        Command::TagInspect(name) => {
            let entry = find_tag(repo, name, cancel)?;
            Ok(Outcome::success(tag_details(
                &repo.inspect_tag(&entry, cancel)?,
            )))
        }
        Command::TagCreate {
            name,
            revision,
            message,
            sign,
            preview,
        } => {
            // Git signs with the repository's configured key and its own agent;
            // no passphrase passes through Rocket.
            let p = match (sign, message) {
                (true, Some(message)) => repo.preview_signed_tag(name, revision, message, cancel)?,
                _ => repo.preview_create_tag(name, revision, message.as_deref(), cancel)?,
            };
            apply_tag(repo, &p, *preview, cancel)
        }
        Command::TagDelete {
            name,
            expected,
            preview,
        } => {
            let entry = find_tag(repo, name, cancel)?;
            if entry.id != ObjectId::parse(expected)? {
                return Err(Error::Tag(TagFailure::StalePreview));
            }
            let p = repo.preview_delete_tag(&entry, cancel)?;
            apply_tag(repo, &p, *preview, cancel)
        }
        Command::TagRemote(remote) => {
            let s = repo.remote_tags(remote.as_bytes(), cancel)?;
            Ok(Outcome::success(json!({"remote":out::bytes(&s.remote),
                "entries":s.entries.iter().map(remote_tag).collect::<Vec<_>>(),"urls_omitted":true})))
        }
        Command::TagPublish { name, remote, source, preview } => {
            let s = repo.remote_tags(remote.as_bytes(), cancel)?;
            let p = repo.preview_remote_tag(&s, name.as_bytes(), RemoteTagAction::Publish, cancel)?;
            if p.entry().local.as_ref().map(|t| t.id.as_str()) != Some(source.as_str()) {
                return Err(Error::Tag(TagFailure::StalePreview));
            }
            apply_remote_tag(repo, &p, *preview, cancel)
        }
        Command::TagDeleteRemote { name, remote, expected, preview } => {
            let s = repo.remote_tags(remote.as_bytes(), cancel)?;
            let p = repo.preview_remote_tag(&s, name.as_bytes(), RemoteTagAction::Delete, cancel)?;
            if p.entry().remote_id.as_ref().map(ObjectId::as_str) != Some(expected.as_str()) {
                return Err(Error::Tag(TagFailure::RemoteChanged));
            }
            apply_remote_tag(repo, &p, *preview, cancel)
        }
        Command::WorktreeList => Ok(Outcome::success(
            json!({"entries":repo.worktrees(cancel)?.iter().map(worktree).collect::<Vec<_>>()}),
        )),
        Command::WorktreeCreate {
            path,
            branch,
            start,
            existing,
            preview,
        } => {
            let path = absolute(path)?;
            let checkout = if *existing {
                WorktreeCheckout::ExistingBranch(branch.clone())
            } else {
                WorktreeCheckout::NewBranch(branch.clone())
            };
            let p = repo.preview_create_worktree(&path, checkout, start, cancel)?;
            apply_worktree(repo, &p, *preview, cancel)
        }
        Command::WorktreeRemove {
            path,
            expected,
            preview,
        } => {
            let path = std::fs::canonicalize(path).map_err(|source| Error::Io {
                action: "resolve worktree path",
                source,
            })?;
            let entry = repo
                .worktrees(cancel)?
                .into_iter()
                .find(|e| std::fs::canonicalize(&e.path).is_ok_and(|p| p == path))
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "The selected path is not a registered worktree of this repository.".into(),
                    )
                })?;
            if entry.head.as_ref() != Some(&ObjectId::parse(expected)?) {
                return Err(Error::Worktree(WorktreeFailure::StalePreview));
            }
            let p = repo.preview_remove_worktree(&entry, cancel)?;
            apply_worktree(repo, &p, *preview, cancel)
        }
        _ => unreachable!("only advanced commands are dispatched here"),
    }
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir()
            .map(|p| p.join(path))
            .map_err(|source| Error::Io {
                action: "resolve destination path",
                source,
            })
    }
}
fn find_tag(repo: &Repository, name: &str, cancel: &CancellationToken) -> Result<TagEntry> {
    repo.find_tag(name.as_bytes(), cancel)?.ok_or_else(|| {
        Error::InvalidInput(
            "The exact tag name does not exist. Run tag list or check the name.".into(),
        )
    })
}
pub(crate) fn commit(c: &Commit) -> Value {
    json!({"id":c.id.as_str(),"subject":out::bytes(c.message.split(|b|*b==b'\n').next().unwrap_or_default())})
}
fn oid_option(id: &Option<ObjectId>) -> Value {
    json!(id.as_ref().map(ObjectId::as_str))
}
fn tag(e: &TagEntry) -> Value {
    json!({"name":out::bytes(&e.name),"id":e.id.as_str(),"object_type":e.object_type,"annotated":e.annotated(),"symbolic":e.symbolic})
}
fn tag_details(d: &TagDetails) -> Value {
    json!({"entry":tag(&d.entry),"target":d.target.as_str(),"target_type":d.target_type,
    "commit":d.commit.as_ref().map(commit),"annotation":d.annotation.as_ref().map(|a|out::bytes(a))})
}
fn worktree(e: &WorktreeEntry) -> Value {
    json!({"path":out::os_path(&e.path),"head":e.head.as_ref().map(ObjectId::as_str),
    "branch":e.branch.as_ref().map(|b|out::bytes(b)),"main":e.main,"bare":e.bare,
    "locked":e.locked.as_ref().map(|b|out::bytes(b)),"prunable":e.prunable.as_ref().map(|b|out::bytes(b))})
}
fn remote_tag(e: &RemoteTagEntry) -> Value {
    json!({"name":out::bytes(&e.name),"remote_id":e.remote_id.as_ref().map(ObjectId::as_str),"peeled":e.peeled.as_ref().map(ObjectId::as_str),
    "symbolic_target":e.symbolic_target.as_deref().map(out::bytes),"local":e.local.as_ref().map(tag),"relation":e.relation()})
}
pub(crate) fn review_only(review: Value) -> Outcome {
    Outcome::success(json!({"preview":review,"operation_started":false}))
}
pub(crate) fn finish<T>(result: &mut Outcome, command: &Result<T>, verification: &Result<()>) {
    result.primary(command);
    if result.error.is_none() {
        result.primary(verification);
    } else {
        result.observe("verification", verification, |_| Value::Null);
    }
}
fn uncertain(result: &mut Outcome, unknown: bool) {
    if unknown {
        result.warnings.push(json!({"code":"remote_outcome_unknown","message":"The remote may already have changed. Inspect it before retrying; cancellation does not undo a push."}));
    }
}
fn apply_tag(
    repo: &Repository,
    p: &TagPreview,
    preview: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let review = json!({"action":p.action().label(),"details":tag_details(p.details()),"recovery_reference":p.recovery_reference()});
    if preview {
        return Ok(review_only(review));
    }
    let r = repo.apply_tag(p, cancel)?;
    let mut result = Outcome::success(
        json!({"preview":review,"command_result":out::observation(&r.result,out::write_output),
        "observed":out::observation(&r.observed,|v|v.as_ref().map_or(Value::Null,tag)),"verification":out::observation(&r.verification,|_|Value::Null),
        "recovery_reference":r.recovery_reference,"failure":r.failure.map(|f|json!({"code":f.code(),"guidance":f.guidance()}))}),
    );
    finish(&mut result, &r.result, &r.verification);
    Ok(result)
}
/// The lease is the advertised remote object (or its absence for a publish); the
/// backend advertises again under write ownership and refuses a moved remote tag.
fn apply_remote_tag(
    repo: &Repository,
    p: &RemoteTagPreview,
    preview: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let review = json!({"action":p.action().label(),"remote":out::bytes(p.remote()),"tag":remote_tag(p.entry()),
        "expected_result":p.expected_result().map(ObjectId::as_str)});
    if preview {
        return Ok(review_only(review));
    }
    let r = repo.apply_remote_tag(p, cancel)?;
    let remote_outcome = match (&r.verification, &r.observed) {
        (Ok(()), _) if p.action() == RemoteTagAction::Publish => "published",
        (Ok(()), _) => "deleted",
        (Err(_), Ok(id)) if id.as_ref() == p.entry().remote_id.as_ref() => "unchanged",
        (Err(_), Ok(_)) => "different",
        (Err(_), Err(_)) => "unknown",
    };
    let mut result = Outcome::success(
        json!({"preview":review,"command_result":out::observation(&r.result,out::write_output),"remote_outcome":remote_outcome,
        "observed":out::observation(&r.observed,oid_option),"verification":out::observation(&r.verification,|_|Value::Null),
        "error_diagnostics":r.error_diagnostics}),
    );
    finish(&mut result, &r.result, &r.verification);
    result.advisory(r.error_diagnostics);
    uncertain(&mut result, remote_outcome == "unknown");
    Ok(result)
}
pub(crate) fn apply_worktree(
    repo: &Repository,
    p: &WorktreePreview,
    preview: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let review = json!({"action":p.label(),"path":out::os_path(p.path()),"target":p.target().as_str(),
        "branch":p.checkout().map(|c|c.branch()),"recovery_reference":p.recovery_reference()});
    if preview {
        return Ok(review_only(review));
    }
    let r = repo.apply_worktree(p, cancel)?;
    let mut result = Outcome::success(
        json!({"preview":review,"command_result":out::observation(&r.command,out::write_output),
        "observed":out::observation(&r.observed,|entries|json!(entries.iter().map(worktree).collect::<Vec<_>>())),
        "verification":out::observation(&r.verification,|_|Value::Null),"destination":out::os_path(&r.destination),
        "retained_workspace":r.retained_workspace.as_ref().map(|p|out::os_path(p)),"retained_metadata":r.retained_metadata.as_ref().map(|p|out::os_path(p)),
        "metadata_backup":r.metadata_backup.as_ref().map(|p|out::os_path(p)),"recovery_reference":r.recovery_reference,
        "failure":r.failure.map(|f|json!({"code":f.code(),"guidance":f.guidance()}))}),
    );
    finish(&mut result, &r.command, &r.verification);
    Ok(result)
}
