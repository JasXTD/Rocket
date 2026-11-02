//! Explicitly permitted metadata only. Paths, remote URLs, signatures and file
//! contents never cross the Lua boundary; no repository write API is exposed.
use rocket_git::{CancellationToken, Change, EntryKind, GitClient, Head, ObjectId, Result};
use serde_json::{Value, json};
use std::path::Path;

pub fn repository_context(client: &GitClient, path: &Path, selected: Option<&ObjectId>, cancel: &CancellationToken) -> Result<Value> {
    cancel.check()?;
    let repo = client.open(path, cancel)?;
    let status = if repo.info().worktree.is_some() { Some(repo.status(cancel)?) } else { None };
    let head = match &status { Some(s) => s.head.clone(), None => repo.head(cancel)? };
    let (branch, target, detached, unborn) = match &head {
        Head::Branch { name, target } => (Some(name.as_slice()), Some(target.as_str()), false, false),
        Head::Unborn { branch } => (Some(branch.as_slice()), None, false, true),
        Head::Detached { target } => (None, Some(target.as_str()), true, false),
    };
    let branch = branch.map(|b| String::from_utf8_lossy(b).chars().filter(|c| !c.is_control()).take(1024).collect::<String>());
    let changes = status.as_ref().map(|s| json!({
        "staged":s.entries.iter().filter(|e| matches!(e.kind, EntryKind::Tracked|EntryKind::RenameOrCopy) && e.index != Change::Unchanged).count(),
        "unstaged":s.entries.iter().filter(|e| matches!(e.kind, EntryKind::Tracked|EntryKind::RenameOrCopy) && e.worktree != Change::Unchanged).count(),
        "untracked":s.entries.iter().filter(|e| e.kind == EntryKind::Untracked).count(),
        "conflicted":s.entries.iter().filter(|e| e.kind == EntryKind::Unmerged).count()
    }));
    if repo.head(cancel)? != head {
        return Err(crate::invalid("integration_repository_changed: The checkout changed while reading context. Refresh the panel before retrying."));
    }
    cancel.check()?;
    Ok(json!({"branch":branch,"head":target,"detached":detached,"unborn":unborn,
        "bare":status.is_none(),"selected_commit":selected.map(ObjectId::as_str),"changes":changes}))
}
