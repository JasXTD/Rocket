//! Git LFS through the desktop's reviewed operations; the installed git-lfs does
//! the transfers. Server addresses stay out of the output because they can carry
//! credentials. Forced unlocks and pruning stay in the desktop.
use crate::{
    advanced::{finish, review_only},
    args::Command,
    output::{self as out, Outcome},
    run::repo_path,
};
use rocket_git::*;
use serde_json::{Value, json};

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::LfsStatus => Ok(Outcome::success(status(&repo.lfs_status(cancel)?))),
        Command::LfsChange { action, preview } => change(repo, action.clone(), *preview, cancel),
        Command::LfsCheckout { path, preview } => change(repo, LfsAction::Checkout(repo_path(path)?), *preview, cancel),
        Command::LfsLocks(remote) => Ok(Outcome::success(locks(&repo.lfs_locks(remote, None, cancel)?))),
        Command::LfsLock { path, remote, unlock, preview } => {
            let action = match unlock {
                None => LfsLockAction::Lock(path.clone()),
                Some(id) => {
                    let lock = repo.lfs_locks(remote, None, cancel)?.entries.into_iter()
                        .find(|l| l.path == *path && l.id == *id)
                        .ok_or_else(|| Error::InvalidInput("No lock with this ID holds the path. Run lfs locks.".into()))?;
                    if !lock.ours {
                        return Err(Error::InvalidInput("This lock belongs to someone else. A forced unlock stays in Rocket's desktop UI.".into()));
                    }
                    LfsLockAction::Unlock { lock, force: false }
                }
            };
            let p = repo.preview_lfs_lock(remote, action, None, cancel)?;
            let review = json!({"action":p.action().label(),"remote":p.remote(),"path":path,"makes_writable":p.makes_writable()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.apply_lfs_lock(&p, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "local":out::observation(&r.local,|_|Value::Null),"verification":out::observation(&r.verified,|_|Value::Null),
                "remote_outcome":r.outcome.code()}));
            finish(&mut o, &r.result, &r.verified);
            if o.error.is_none() {
                o.primary(&r.local);
            }
            o.data["locks"] = o.observe("lfs_locks", &r.observed, locks);
            if r.outcome == LfsLockOutcome::Unknown {
                o.warnings.push(json!({"code":"remote_outcome_unknown","message":"The lock request may have reached the server. Run lfs locks before retrying."}));
            }
            Ok(o)
        }
        _ => unreachable!("only LFS commands are dispatched here"),
    }
}

fn change(repo: &Repository, action: LfsAction, preview: bool, cancel: &CancellationToken) -> Result<Outcome> {
    let p = repo.preview_lfs(action, None, cancel)?;
    let review = json!({"action":p.action().label(),"head":out::head(p.head()),"objects":p.object_count(),"bytes":p.total_bytes(),
        "files":p.entries().iter().map(entry).collect::<Vec<_>>(),"local_source":p.local_source().is_some(),
        "attributes":p.attributes().map(|a|json!({"removed":a.removed(),"added":a.added()}))});
    if preview {
        return Ok(review_only(review));
    }
    let r = repo.apply_lfs(&p, cancel)?;
    let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
        "verification":out::observation(&r.verification,|_|Value::Null),"upload_outcome":r.upload_outcome.map(LfsUploadOutcome::code),
        "completed_steps":r.completed_steps,"recovery_directory":r.recovery_directory.as_deref().map(out::os_path)}));
    finish(&mut o, &r.result, &r.verification);
    o.data["lfs"] = o.observe("lfs_status", &r.observed, status);
    Ok(o)
}
fn entry(e: &LfsEntry) -> Value {
    json!({"path":out::bytes(e.path.as_bytes()),"oid":e.object.oid,"size":e.object.size,"cached":e.cached,
        "state":match e.state {LfsFileState::Pointer=>"pointer",LfsFileState::Content=>"content",LfsFileState::Missing=>"missing",LfsFileState::Unsupported=>"unsupported"}})
}
fn status(s: &LfsStatus) -> Value {
    let (tracking, tracking_error) = match &s.tracking {
        Ok(rules) => (json!(rules.iter().map(|r| json!({"line":r.line,"text":r.text})).collect::<Vec<_>>()), Value::Null),
        Err(message) => (Value::Null, json!(message)),
    };
    json!({"version":s.version,"head":out::head(&s.head),"files":s.entries.iter().map(entry).collect::<Vec<_>>(),
        "remotes":s.remotes,"tracking":tracking,"tracking_error":tracking_error})
}
fn locks(l: &LfsLocks) -> Value {
    json!({"remote":l.remote,"locks":l.entries.iter().map(|k|json!({"id":k.id,"path":k.path,"owner":k.owner,"locked_at":k.locked_at,"ours":k.ours})).collect::<Vec<_>>()})
}
