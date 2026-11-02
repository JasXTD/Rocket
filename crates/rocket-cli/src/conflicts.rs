//! Conflict resolution through the desktop editor's checked saves. The working
//! file is pinned across invocations by its SHA-256 and a kept version by its blob
//! ID; saves recheck the file and conflict stages under repository write ownership
//! and retain recovery copies. Nothing here finishes the merge or rebase.
use crate::{
    advanced::review_only,
    args::Command,
    output::{self as out, Outcome, Problem},
    run::{repo_path, write},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
    Both,
}
#[derive(Debug)]
pub enum Choice {
    /// One-based conflict numbers, as `conflict show` lists them.
    Blocks(Vec<(usize, Side)>),
    All(Side),
    /// A complete UTF-8 result; line endings and BOM follow the conflicted file.
    Result(PathBuf),
}

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::ConflictShow(path) => {
            let path = repo_path(path)?;
            let whole = repo.review_whole_conflict(&path, cancel)?;
            // Binary, LFS, submodule and deleted versions are kept whole instead.
            let text = if whole.kind() == ConflictKind::File && !whole.binary() && whole.working_size().is_some() {
                match repo.edit_conflict(&path, cancel) {
                    Ok(p) => Some(p),
                    Err(Error::Conflict(ConflictFailure::UnsupportedFile)) => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };
            Ok(Outcome::success(show(&whole, text.as_ref())?))
        }
        Command::ConflictResolve { path, expected, choice, stage, preview } => {
            let p = repo.edit_conflict(&repo_path(path)?, cancel)?;
            if sha256_bytes(p.original())? != *expected {
                return Ok(Outcome::failure(Problem::new(
                    "conflict_file_changed",
                    "The conflicted file changed after it was reviewed.",
                    "Run conflict show again and review the current file. Nothing was written.",
                    1,
                )));
            }
            let text = match resolved(&p, choice)? {
                Ok(text) => text,
                Err(problem) => return Ok(Outcome::failure(problem)),
            };
            let review = json!({"path":out::bytes(p.path().as_bytes()),"sha256":expected,"stage":stage,
                "remaining_conflicts":conflict_blocks(&text).blocks.len(),"result":out::bytes(text.as_bytes())});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.save_conflict(&p, &text, *stage, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "saved":r.saved,"staged":r.staged,"recovery_directory":r.recovery_directory.as_deref().map(out::os_path)}));
            o.primary(&r.result);
            o.data["status"] = o.observe("status", &r.status, out::status);
            o.data["operation"] = o.observe("operation", &r.state, out::operation);
            Ok(o)
        }
        Command::ConflictKeep { path, keep, expected, preview } => {
            let p = repo.review_whole_conflict(&repo_path(path)?, cancel)?;
            if !p.can_keep(*keep) {
                return Err(Error::Conflict(ConflictFailure::VersionUnavailable));
            }
            let kept = match keep {
                ConflictKeep::Ours => p.version(2),
                ConflictKeep::Theirs => p.version(3),
                ConflictKeep::Deletion => None,
            };
            if kept.map(|v| v.id.as_str()) != expected.as_deref() {
                return Err(Error::Conflict(ConflictFailure::ReviewStale));
            }
            let review = json!({"path":out::bytes(p.path().as_bytes()),"side":keep.label(),"kept":kept.map(version),
                "planned_change":p.planned_change(*keep).map(WholeFileChange::label)});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.keep_conflict_side(&p, *keep, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "file_change":r.file_change.map(WholeFileChange::label),"staged":r.staged,
                "recovery_directory":r.recovery_directory.as_deref().map(out::os_path)}));
            o.primary(&r.result);
            o.data["status"] = o.observe("status", &r.status, out::status);
            o.data["operation"] = o.observe("operation", &r.state, out::operation);
            Ok(o)
        }
        Command::ConflictMarkResolved(path) => Ok(write(repo.mark_resolved(&repo_path(path)?, cancel)?)),
        _ => unreachable!("only conflict commands are dispatched here"),
    }
}

/// Apply choices the way the editor's Use ours / Use theirs / Use both do: the
/// block becomes its ours lines, its theirs lines, or ours then theirs.
fn resolved(p: &ConflictPreview, choice: &Choice) -> Result<std::result::Result<String, Problem>> {
    let working = p.working();
    let blocks = conflict_blocks(working);
    let mut chosen = match choice {
        Choice::Result(file) => {
            let bytes = std::fs::read(file).map_err(|source| Error::Io { action: "read the conflict result file", source })?;
            let text = String::from_utf8(bytes).map_err(|_| Error::Conflict(ConflictFailure::UnsupportedFile))?;
            // The save restores the original file's BOM; a second one would become text.
            return Ok(Ok(text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text)));
        }
        Choice::All(side) => (1..=blocks.blocks.len()).map(|n| (n, *side)).collect(),
        Choice::Blocks(list) => list.clone(),
    };
    if blocks.malformed {
        return Ok(Err(Problem::new(
            "conflict_markers_malformed",
            "This file has incomplete or nested conflict markers, so conflicts cannot be chosen by number.",
            "Write the complete result to a file and use --result FILE, or resolve the file in Rocket's conflict editor.",
            1,
        )));
    }
    if let Some((number, _)) = chosen.iter().find(|(n, _)| *n > blocks.blocks.len()) {
        return Err(Error::InvalidInput(format!(
            "This file has {} conflict(s); conflict {number} does not exist. Run conflict show.",
            blocks.blocks.len()
        )));
    }
    // Replace from the last block back, so earlier byte ranges stay valid.
    chosen.sort_by_key(|&(number, _)| std::cmp::Reverse(number));
    let mut text = working.to_owned();
    for (number, side) in chosen {
        let block = &blocks.blocks[number - 1];
        let mut replacement = String::new();
        if side != Side::Theirs {
            replacement.push_str(&working[block.ours.clone()]);
        }
        if side != Side::Ours {
            replacement.push_str(&working[block.theirs.clone()]);
        }
        text.replace_range(block.range.clone(), &replacement);
    }
    Ok(Ok(text))
}
fn line_at(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset].iter().filter(|b| **b == b'\n').count() + 1
}
fn version(v: &ConflictVersion) -> Value {
    json!({"id":v.id.as_str(),"size":v.size,"executable":v.executable,"binary":v.binary,"submodule":v.gitlink,
        "lfs":v.lfs.as_ref().map(|l|json!({"oid":l.oid,"size":l.size})),"unavailable":v.unavailable.map(LfsFailure::code)})
}
fn show(whole: &WholeConflictPreview, text: Option<&ConflictPreview>) -> Result<Value> {
    let text = match text {
        None => Value::Null,
        Some(p) => {
            let working = p.working();
            let blocks = conflict_blocks(working);
            json!({"sha256":sha256_bytes(p.original())?,"crlf":p.crlf(),"bom":p.bom(),"malformed":blocks.malformed,
                "working":out::bytes(working.as_bytes()),
                "conflicts":blocks.blocks.iter().enumerate().map(|(i,b)|json!({"number":i+1,
                    "start_line":line_at(working,b.range.start),"end_line":line_at(working,b.range.end.saturating_sub(1)),
                    "ours":out::bytes(working[b.ours.clone()].as_bytes()),"theirs":out::bytes(working[b.theirs.clone()].as_bytes())})).collect::<Vec<_>>()})
        }
    };
    Ok(json!({"path":out::bytes(whole.path().as_bytes()),
        "kind":match whole.kind() {ConflictKind::File=>"file",ConflictKind::Lfs=>"lfs",ConflictKind::Submodule=>"submodule"},
        "binary":whole.binary(),"deleted":whole.deleted(),
        "versions":{"base":whole.version(1).map(version),"ours":whole.version(2).map(version),"theirs":whole.version(3).map(version)},
        "working":{"size":whole.working_size(),"binary":whole.working_binary(),
            "matches":{"base":whole.working_matches(1),"ours":whole.working_matches(2),"theirs":whole.working_matches(3)}},
        "can_keep":{"ours":whole.can_keep(ConflictKeep::Ours),"theirs":whole.can_keep(ConflictKeep::Theirs),"deletion":whole.can_keep(ConflictKeep::Deletion)},
        "text":text}))
}
