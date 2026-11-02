//! Repository maintenance through the desktop's reviewed operations: remote rename
//! and tracking-ref cleanup, server branch deletion with an exact lease, branch
//! fast-forwards, linked-worktree maintenance and the repository-local identity.
use crate::{
    advanced::{absolute, apply_worktree, review_only},
    args::Command,
    output::{self as out, Outcome, Problem},
    run::remote,
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::Path;

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::RemoteRename { old, new, preview } => {
            let current = repo.remotes(cancel)?.into_iter().find(|r| r.name == old.as_bytes())
                .ok_or_else(|| Error::InvalidInput("Remote not found; use remote list.".into()))?;
            let p = repo.preview_remote_rename(&current, new.as_bytes(), cancel)?;
            let review = json!({"remote":out::bytes(&p.remote().name),"name":out::bytes(p.name()),
                "tracking_branches":p.affected_branches().iter().map(|b|out::bytes(&b.reference)).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.rename_remote(&p, cancel)?;
            let mut o = remote(r.report);
            o.data["preview"] = review;
            o.data["config_backup"] = out::os_path(&r.config_backup);
            o.data["recovery_reference"] = json!(r.recovery_refs);
            Ok(o)
        }
        Command::RemotePrune { remote: name, refs, preview } => {
            let p = repo.preview_remote_cleanup(name.as_bytes(), cancel)?;
            let review = json!({"remote":out::bytes(p.remote()),
                "obsolete":p.obsolete().iter().map(|r|json!({"reference":out::bytes(&r.reference),"id":r.id.as_str()})).collect::<Vec<_>>(),
                "stale_local_branches":p.stale_local_branches().iter().map(|b|out::bytes(b)).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            let selected: Vec<Vec<u8>> = refs.iter().map(|r| r.as_bytes().to_vec()).collect();
            Ok(cleanup(review, repo.prune_tracking_refs(&p, &selected, cancel)?))
        }
        Command::BranchDeleteRemote { remote: name, branch, expected, preview } => {
            let p = repo.preview_remote_branch_delete(name.as_bytes(), branch.as_bytes(), cancel)?;
            if p.expected().as_str() != expected {
                return Ok(Outcome::failure(Problem::new("remote_branch_changed", "The server branch is not at the reviewed object.",
                    "Fetch, then pass the remote branch's target from branch list. Nothing was deleted.", 1)));
            }
            let review = json!({"remote":out::bytes(p.remote()),"branch":out::bytes(p.branch()),"expected":p.expected().as_str()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.delete_remote_branch(&p, cancel)?;
            let outcome = match r.remote_deleted {
                Some(true) => "deleted",
                Some(false) => "unchanged",
                None => "unknown",
            };
            let mut o = cleanup(review, r);
            o.data["remote_outcome"] = json!(outcome);
            if outcome == "unknown" {
                o.warnings.push(json!({"code":"remote_outcome_unknown","message":"The server branch could not be read again. Check it before retrying."}));
            }
            Ok(o)
        }
        Command::BranchFastForward { branches, preview } => {
            let p = repo.preview_fast_forward_branches(cancel)?;
            let review = json!({"branches":p.entries().iter().map(|e|json!({"branch":out::bytes(&e.branch),"before":e.before.as_str(),
                "after":e.after.as_ref().map(ObjectId::as_str),"upstream":e.upstream.as_deref().map(out::bytes),"blocked":e.blocked})).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            let selected: Vec<Vec<u8>> = branches.iter()
                .map(|b| if b.starts_with("refs/") { b.as_bytes().to_vec() } else { format!("refs/heads/{b}").into_bytes() })
                .collect();
            let r = repo.fast_forward_branches(&p, &selected, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "recovery_reference":r.recovery}));
            o.primary(&r.result);
            o.data["branches"] = o.observe("branches", &r.observed, |b| out::branches(b));
            Ok(o)
        }
        Command::WorktreeMaintain { kind, path, destination, expected, preview } => {
            let entry = find_worktree(repo, path, cancel)?;
            if entry.head.as_ref().map(ObjectId::as_str) != Some(expected.as_str()) {
                return Err(Error::Worktree(WorktreeFailure::StalePreview));
            }
            // Pruning has no destination; the backend ignores it.
            let destination = destination.as_deref().map(absolute).transpose()?.unwrap_or_default();
            let p = repo.preview_maintain_worktree(*kind, &entry, &destination, cancel)?;
            apply_worktree(repo, &p, *preview, cancel)
        }
        Command::IdentityShow => Ok(Outcome::success(identity(&repo.identity(cancel)?))),
        Command::IdentitySet { name, email, preview } => {
            let current = repo.identity(cancel)?;
            let wanted = LocalIdentity {
                name: name.clone().or_else(|| current.local.name.clone()),
                email: email.clone().or_else(|| current.local.email.clone()),
            };
            wanted.validate()?;
            let review = json!({"current":identity(&current),"local":person(&wanted)});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.set_identity(&current, &wanted, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,out::write_output)}));
            o.primary(&r.result);
            o.data["identity"] = o.observe("identity", &r.identity, identity);
            Ok(o)
        }
        _ => unreachable!("only maintenance commands are dispatched here"),
    }
}

fn cleanup(review: Value, r: RemoteCleanupReport) -> Outcome {
    let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
        "recovery_reference":r.recovery}));
    o.primary(&r.result);
    o
}
/// A registered folder that no longer exists (repair, prune) is matched by its
/// spelling; one that exists is matched by its resolved path.
fn find_worktree(repo: &Repository, path: &Path, cancel: &CancellationToken) -> Result<WorktreeEntry> {
    let wanted = absolute(path)?;
    let resolved = std::fs::canonicalize(&wanted).ok();
    let spelling = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    repo.worktrees(cancel)?
        .into_iter()
        .find(|e| match (&resolved, std::fs::canonicalize(&e.path).ok()) {
            (Some(a), Some(b)) => *a == b,
            _ => spelling(&e.path) == spelling(&wanted),
        })
        .ok_or_else(|| Error::InvalidInput("The selected path is not a registered worktree of this repository.".into()))
}
fn person(p: &LocalIdentity) -> Value {
    json!({"name":p.name,"email":p.email})
}
fn identity(i: &RepositoryIdentity) -> Value {
    json!({"local":person(&i.local),"effective":person(&i.effective),"inherited":person(&i.inherited)})
}
