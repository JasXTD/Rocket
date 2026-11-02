//! Line and file selections from one change or one stash. A review's patch is
//! pinned across invocations by its SHA-256; the backend compares the files again
//! under repository write ownership immediately before it writes.
use crate::{
    advanced::{finish, review_only},
    args::Command,
    output::{self as out, Outcome, Problem},
    run::{repo_path, write},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::ffi::OsStr;

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::Hunks { path, staged } => {
            let (path, review, untracked) = change(repo, path, *staged, cancel)?;
            let patch = review.patch();
            let lines: Vec<_> = patch.split(|b| *b == b'\n').collect();
            let changed = review.changed_lines();
            Ok(Outcome::success(json!({"path":out::bytes(path.as_bytes()),"staged":staged,"untracked":untracked,
                "patch":out::bytes(patch),"patch_sha256":sha256_bytes(patch)?,"hunks":hunks(&lines,&changed),
                "changed_lines":changed.iter().map(|&id|json!({"id":id,"sign":if lines[id].starts_with(b"+") {"+"} else {"-"},"text":out::bytes(&lines[id][1..])})).collect::<Vec<_>>(),
                "selection_unavailable":review.selection_unavailable(),"selection_note":review.selection_note()})))
        }
        Command::StageLines { path, lines, expected, unstage } => {
            let (path, review, _) = change(repo, path, *unstage, cancel)?;
            if sha256_bytes(review.patch())? != *expected {
                return Ok(stale("hunks"));
            }
            let mut o = write(repo.apply_review(&review, lines, cancel)?);
            o.data["selection"] = json!({"path":out::bytes(path.as_bytes()),"lines":lines});
            Ok(o)
        }
        Command::StashShow { id, part } => {
            let entry = find_stash(repo, id, cancel)?;
            let review = repo.review_stash_apply(&entry, *part, cancel)?;
            Ok(Outcome::success(json!({"stash":stash_entry(&entry),"part":part_name(*part),
                "files":review.files().iter().map(stash_file).collect::<Result<Vec<_>>>()?})))
        }
        Command::StashApplySelection { id, part, paths, hunks, lines, expected, preview } => {
            let entry = find_stash(repo, id, cancel)?;
            let review = repo.review_stash_apply(&entry, *part, cancel)?;
            let mut selected = Vec::new();
            for path in paths {
                let path = repo_path(path)?;
                let (file, saved) = review.files().iter().enumerate().find(|(_, f)| *f.path() == path).ok_or_else(|| {
                    Error::InvalidInput("This part of the stash does not change a selected path. Run stash show.".into())
                })?;
                if let Some(expected) = expected
                    && sha256_bytes(saved.patch())? != *expected
                {
                    return Ok(stale("stash show"));
                }
                selected.push(StashApplySelection { file, hunks: hunks.clone(), lines: lines.clone() });
            }
            let p = repo.preview_stash_apply(&review, &selected, cancel)?;
            let review = json!({"stash":stash_entry(p.entry()),"part":part_name(*part),"files":p.files().iter().map(file_edit).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            Ok(patch_report(review, repo.apply_stash_selection(&p, cancel)?))
        }
        Command::StashSaveLines { message, path, lines, expected, preview } => {
            let (path, review, _) = change(repo, path, false, cancel)?;
            if sha256_bytes(review.patch())? != *expected {
                return Ok(stale("hunks"));
            }
            let p = repo.preview_stash_selection(&review, lines, message, cancel)?;
            let review = json!({"path":out::bytes(path.as_bytes()),"lines":lines,"selected_count":p.selected_count(),
                "files":p.files().iter().map(file_edit).collect::<Vec<_>>()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.stash_selection(&p, cancel)?;
            // The selection is backed up as `<namespace>/0` before Git stores it.
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "stash":r.stash.as_ref().map(ObjectId::as_str),"recovery_reference":r.stash.as_ref().map(|_|format!("{}/0",r.recovery)),
                "cleanup":r.cleanup.as_ref().map(patch_json)}));
            o.primary(&r.result);
            o.data["stashes"] = o.observe("stashes", &r.stashes, |s| out::stashes(s));
            Ok(o)
        }
        Command::StashPop { index, expected, preview } => {
            let id = ObjectId::parse(expected)?;
            let entry = repo.stashes(cancel)?.into_iter().find(|s| s.index == *index && s.id == id).ok_or_else(|| {
                Error::InvalidInput("Stash selection changed; list stashes again and use the expected full ID.".into())
            })?;
            let p = repo.preview_stash_control(StashControl::Pop(entry.clone()), cancel)?;
            let review = json!({"action":"pop","stash":stash_entry(&entry),"stash_count":p.entries().len()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.apply_stash_control(&p, cancel)?;
            // Every listed stash is backed up under the namespace before anything changes.
            // Name the popped one's backup only once it is confirmed to hold that stash.
            let backup = format!("{}/{}", r.recovery, entry.index);
            let kept = repo.resolve_commit(&backup, &CancellationToken::new()).is_ok_and(|id| id == entry.id);
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "removed":r.removed,"recovery_reference":kept.then_some(backup),"recovery_namespace":r.recovery}));
            o.primary(&r.result);
            o.data["stashes"] = o.observe("stashes", &r.stashes, |s| out::stashes(s));
            o.data["status"] = o.observe("status", &r.status, out::status);
            Ok(o)
        }
        _ => unreachable!("only selection commands are dispatched here"),
    }
}

/// An untracked file has no index entry to compare with, so its review reads the
/// working file itself; only selected additions can be staged from it.
fn change(
    repo: &Repository,
    path: &OsStr,
    staged: bool,
    cancel: &CancellationToken,
) -> Result<(RepoPath, ChangeReview, bool)> {
    let path = repo_path(path)?;
    let untracked = !staged
        && repo.status(cancel)?.entries.iter().any(|e| e.path == path && e.kind == EntryKind::Untracked);
    let review = if untracked { repo.review_untracked(&path, cancel)? } else { repo.review_change(&path, staged, cancel)? };
    Ok((path, review, untracked))
}
/// Changed-line IDs are zero-based lines of the raw patch, in patch order; group
/// them under the `@@` header that precedes them.
fn hunks(lines: &[&[u8]], changed: &[usize]) -> Vec<Value> {
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    let (mut header, mut next) = (0, 0);
    for &id in changed {
        while next <= id {
            if lines[next].starts_with(b"@@") {
                header = next;
            }
            next += 1;
        }
        match groups.last_mut() {
            Some((h, ids)) if *h == header => ids.push(id),
            _ => groups.push((header, vec![id])),
        }
    }
    groups.into_iter().map(|(h, ids)| json!({"header":out::bytes(lines[h]),"line_ids":ids})).collect()
}
fn stale(review: &str) -> Outcome {
    Outcome::failure(Problem::new(
        "selection_stale_preview",
        "The change no longer matches the reviewed patch.",
        &format!("Run {review} again and choose IDs from the new review. Nothing was written."),
        1,
    ))
}
fn find_stash(repo: &Repository, id: &str, cancel: &CancellationToken) -> Result<StashEntry> {
    let id = ObjectId::parse(id)?;
    repo.stashes(cancel)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| Error::InvalidInput("No stash has this full ID. Run stash list.".into()))
}
fn stash_entry(s: &StashEntry) -> Value {
    json!({"index":s.index,"id":s.id.as_str(),"subject":out::bytes(&s.subject)})
}
fn part_name(part: StashPart) -> &'static str {
    match part {
        StashPart::Tracked => "tracked",
        StashPart::Index => "index",
        StashPart::Untracked => "untracked",
    }
}
fn stash_file(f: &StashApplyFile) -> Result<Value> {
    let headers = f.hunk_headers();
    Ok(json!({"path":out::bytes(f.path().as_bytes()),"original_path":f.original_path().map(|p|out::bytes(p.as_bytes())),
        "patch":out::bytes(f.patch()),"patch_sha256":sha256_bytes(f.patch())?,"whole_only":f.whole_only(),"mode_change":f.mode_change(),
        "hunks":headers.iter().enumerate().map(|(i,h)|json!({"index":i,"header":h,"line_ids":f.hunk_line_ids(i)})).collect::<Vec<_>>()}))
}
fn file_edit(f: &FileEditPreview) -> Value {
    json!({"path":out::bytes(f.path().as_bytes()),"before_size":f.before().map(<[u8]>::len),"after_size":f.after().map(<[u8]>::len)})
}
fn patch_json(r: &PatchReport) -> Value {
    json!({"command_result":out::observation(&r.result,|_|Value::Null),"verification":out::observation(&r.verification,|_|Value::Null),
        "changed":r.changed.iter().map(|p|out::bytes(p.as_bytes())).collect::<Vec<_>>(),
        "recovery":r.recovery.iter().map(|p|out::os_path(p)).collect::<Vec<_>>()})
}
fn patch_report(review: Value, r: PatchReport) -> Outcome {
    let mut o = Outcome::success(patch_json(&r));
    o.data["preview"] = review;
    finish(&mut o, &r.result, &r.verification);
    o.data["status"] = o.observe("status", &r.status, out::status);
    o
}
