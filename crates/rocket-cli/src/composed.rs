//! Composed writes retain the backend's write ownership and recovery records.
//! A pin binds separate preview/apply invocations; it never bypasses revalidation.
use crate::{
    advanced::review_only,
    args::Command,
    output::{self as out, Outcome},
};
use rocket_git::*;
use serde_json::json;

fn check_review(expected: &Option<String>, actual: &str) -> Result<()> {
    if expected.as_deref() != Some(actual) {
        return Err(Error::InvalidInput("workflow_review_stale: the reviewed files, checkout, message or destination changed. Preview again; nothing was written.".into()));
    }
    Ok(())
}

fn observations(repo: &Repository, o: &mut Outcome) {
    // Cancellation is not rollback. Observe with a fresh token, without retrying
    // the write or claiming that local state proves a remote result.
    let probe = CancellationToken::new();
    o.data["status"] = o.observe("status", &repo.status(&probe), out::status);
    o.data["operation"] = o.observe("operation", &repo.operation_state(&probe), out::operation);
}

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::CommitPush {
            remote,
            destination,
            message,
            review,
            preview,
        } => {
            let p = repo.preview_commit_push(
                message,
                remote.as_bytes(),
                destination.as_bytes(),
                cancel,
            )?;
            let digest = p.review_digest()?;
            let reviewed = json!({"review_sha256":digest,"head":out::head(p.head()),
                "remote":out::bytes(p.remote()),"branch":out::bytes(p.branch()),
                "destination":out::bytes(p.destination()),"message":p.message(),"staged_files":p.staged_files()});
            if *preview {
                return Ok(review_only(reviewed));
            }
            check_review(review, &digest)?;
            let r = repo.apply_commit_push(&p, cancel)?;
            let mut o = Outcome::success(json!({"preview":reviewed,"phase":r.phase,
                "commit":r.commit.as_ref().map(ObjectId::as_str),"reviewed_commit_confirmed":r.reviewed_commit_confirmed,
                "push":null,"recovery":{"local_commit_retained":r.commit.is_some(),
                "guidance":"Inspect the retained commit and remote outcome. Retry Push separately if needed; do not repeat Commit & Push."}}));
            o.primary(&r.result);
            if let Some(push) = r.push {
                let pushed = crate::run::push(push);
                o.data["push"] = json!({"ok":pushed.error.is_none(),"data":pushed.data,
                    "error":pushed.error.as_ref().map(|e|&e.value)});
                o.warnings.extend(pushed.warnings);
                // Keep cancellation exit semantics, otherwise prefer the specific
                // push failure over the composed operation's incomplete summary.
                if !matches!(&r.result, Err(Error::Cancelled | Error::TimedOut))
                    && pushed.error.is_some()
                {
                    o.error = pushed.error;
                }
            }
            observations(repo, &mut o);
            Ok(o)
        }
        Command::SwitchSaved {
            branch,
            review,
            preview,
        } => {
            let p = repo.preview_saved_work(SavedWorkAction::Switch(branch.clone()), cancel)?;
            let digest = p.review_digest()?;
            let reviewed = json!({"review_sha256":digest,"head":out::head(p.head()),"branch":branch,
                "target":p.target().as_str(),"changed_files":p.changed_files(),"untracked":"kept in place; may block switch",
                "recovery_stash":"retained after restore; never automatically dropped"});
            if *preview {
                return Ok(review_only(reviewed));
            }
            check_review(review, &digest)?;
            let r = repo.apply_saved_work(&p, cancel)?;
            let mut o = Outcome::success(json!({"preview":reviewed,"phase":r.phase,
                "recovery":{"stash":r.stash.as_ref().map(ObjectId::as_str),"journal":out::os_path(&r.journal),
                "guidance":"Inspect status and the retained stash. Resolve any restore conflicts before applying or dropping the stash; do not blindly repeat the switch."}}));
            o.primary(&r.result);
            observations(repo, &mut o);
            Ok(o)
        }
        _ => unreachable!("composed command dispatch"),
    }
}
