//! History writes through the desktop's reviewed operations: Undo of a commit this
//! CLI made, cherry-pick, revert and amend with their recovery, rebase start (with a
//! data-only plan) and reset. Pins name the reviewed commits; the backend rechecks
//! the branch, index and operation under repository write ownership.
use crate::{
    advanced::{finish, review_only},
    args::Command,
    output::{self as out, Outcome, Problem},
    run::{rebase, write},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::Path;

/// `commit` reports what `undo-commit` needs. A missing Undo never changes the
/// commit's own result.
pub fn commit(report: UndoableCommitReport) -> Outcome {
    let mut o = write(report.write);
    let undo = report.undo.as_ref().and_then(|u| {
        u.index_digest().ok().map(|digest| json!({"branch":out::bytes(u.branch()),"commit":u.commit().as_str(),
            "previous":u.previous().map(ObjectId::as_str),"index_sha256":digest}))
    });
    // The backend explains its own refusals; a failed digest has no message of its own.
    let unavailable = report.undo_unavailable.or_else(|| {
        (report.undo.is_some() && undo.is_none()).then(|| "Commit completed; optional Undo could not be prepared.".to_owned())
    });
    o.data["undo"] = undo.unwrap_or(Value::Null);
    if let Some(message) = unavailable {
        o.warnings.push(json!({"code":"undo_unavailable","message":message}));
    }
    o
}

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::UndoCommit { branch, expected, index, preview } => {
            let record = repo.pinned_undo_commit(branch.as_bytes(), &ObjectId::parse(expected)?, index, cancel)?;
            let p = repo.preview_undo_commit(&record, cancel)?;
            let review = json!({"branch":out::bytes(record.branch()),"commit":record.commit().as_str(),
                "previous":record.previous().map(ObjectId::as_str),"subject":out::bytes(record.subject()),
                "recovery_reference":p.recovery_reference()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.apply_undo_commit(&p, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,|_|Value::Null),
                "verification":out::observation(&r.verified,|_|Value::Null),"recovery_reference":r.recovery_reference}));
            finish(&mut o, &r.result, &r.verified);
            o.data["status"] = o.observe("status", &r.status, out::status);
            Ok(o)
        }
        Command::CommitAction { request, expected, message, preview } => {
            let p = repo.preview_commit_action(request.clone(), cancel)?;
            if let Some(expected) = expected {
                // Amend pins the commit it replaces; cherry-pick and revert can pin HEAD.
                let pinned = if request.operation == CommitOperation::Amend {
                    Some(p.source().id.as_str())
                } else {
                    target(p.head())
                };
                if pinned != Some(expected.as_str()) {
                    return Err(Error::CommitAction(CommitFailure::StalePreview));
                }
            }
            let review = json!({"operation":request.label(),"head":out::head(p.head()),"mainline":request.mainline,
                "commits":p.commits().iter().map(crate::advanced::commit).collect::<Vec<_>>(),"sequence":p.is_sequence(),
                "remaining":p.remaining().iter().map(ObjectId::as_str).collect::<Vec<_>>(),
                "applied":p.applied().iter().map(crate::advanced::commit).collect::<Vec<_>>(),
                "prediction":p.prediction().map(out::prediction),"recovery_reference":p.recovery_reference()});
            if *preview {
                return Ok(review_only(review));
            }
            let mut o = commit_report(repo.apply_commit_action(&p, message.as_deref(), cancel)?);
            o.data["preview"] = review;
            Ok(o)
        }
        Command::RebaseStart { target: onto, interactive, plan, update_refs, preserve_merges, expected, preview } => {
            let mut p = if *preserve_merges {
                repo.preview_rebase_preserving_merges(onto, cancel)?
            } else {
                let action = if *interactive { RebaseAction::Interactive } else { RebaseAction::Start };
                repo.preview_rebase(action, onto, cancel)?
            };
            if *update_refs {
                p = p.with_update_refs(true)?;
            }
            if let Some(file) = plan {
                p = p.with_plan(read_plan(file)?)?;
            }
            if let Some((original, onto)) = expected
                && (p.original().as_str() != original || p.onto().map(ObjectId::as_str) != Some(onto.as_str()))
            {
                return Err(Error::Rebase(RebaseFailure::StalePreview));
            }
            let review = json!({"action":p.action().label(),"branch":out::bytes(p.branch()),"original":p.original().as_str(),
                "onto":p.onto().map(ObjectId::as_str),"commits":p.commits().iter().map(crate::advanced::commit).collect::<Vec<_>>(),
                "fast_forward":p.fast_forward(),"up_to_date":p.up_to_date(),"preserves_merges":p.preserves_merges(),
                "update_refs":p.update_refs(),"dependents":p.dependents().iter().map(|b|json!({"reference":out::bytes(&b.reference),
                    "target":b.target.as_str(),"checked_out_at":b.checked_out_at.as_deref().map(out::os_path),"moves":b.moves()})).collect::<Vec<_>>(),
                "plan":p.plan().map(|rows|rows.iter().map(|r|json!({"commit":r.commit.as_str(),"instruction":r.instruction.label(),
                    "message":r.message,"stop_after":r.stop_after})).collect::<Vec<_>>()),
                "prediction":p.prediction().map(out::prediction),"recovery_reference":p.recovery_reference()});
            if *preview {
                return Ok(review_only(review));
            }
            let mut o = rebase(repo.apply_rebase(&p, cancel)?);
            o.data["preview"] = review;
            Ok(o)
        }
        Command::Reset { mode, target: commit, expected, preview } => {
            let p = repo.preview_recovery(&ObjectId::parse(commit)?, RecoveryAction::Reset(*mode), cancel)?;
            if target(p.head()) != Some(expected.as_str()) {
                return Err(Error::Recovery(RecoveryFailure::StalePreview));
            }
            let review = json!({"action":mode.label(),"target":crate::advanced::commit(p.target()),"head":out::head(p.head()),
                "relation":p.relation().map(|(target_only,head_only)|json!({"target_only":target_only,"head_only":head_only})),
                "recovery_reference":p.recovery_reference()});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.apply_recovery(&p, cancel)?;
            let mut o = write(r.write);
            o.data["preview"] = review;
            if o.error.is_none() {
                o.primary(&r.verification);
            }
            o.data["verification"] = out::observation(&r.verification, |_| Value::Null);
            o.data["branches"] = o.observe("branches", &r.branches, |b| out::branches(b));
            o.data["recovery_reference"] = json!(r.recovery_reference);
            if let Some(f) = r.failure
                && o.error.as_ref().is_some_and(|e| e.exit != 130 && e.exit != 124)
            {
                o.error = Some(Problem::new(f.code(), "Reset needs attention.", f.guidance(), 1));
            }
            Ok(o)
        }
        _ => unreachable!("only history writes are dispatched here"),
    }
}

fn target(head: &Head) -> Option<&str> {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => Some(target.as_str()),
        Head::Unborn { .. } => None,
    }
}
fn commit_report(r: CommitWriteReport) -> Outcome {
    let mut o = write(r.write);
    o.data["branches"] = o.observe("branches", &r.branches, |b| out::branches(b));
    o.data["operation"] = o.observe("operation", &r.state, out::operation);
    o.data["recovery_reference"] = json!(r.recovery_reference);
    o.data["journal_cleanup"] = o.observe("journal_cleanup", &r.journal_cleanup, |_| Value::Null);
    if let Some(f) = r.failure {
        o.data["commit_failure"] = json!({"code":f.code(),"guidance":f.guidance()});
        if o.error.as_ref().is_some_and(|e| e.exit != 130 && e.exit != 124) {
            o.error = Some(Problem::new(f.code(), "The commit operation needs attention.", f.guidance(), 1));
        }
    }
    o
}
/// A data-only plan: one row per reviewed commit, in replay order. Rows that run
/// commands (`exec`) are refused; such plans are reviewed in the desktop UI.
fn read_plan(file: &Path) -> Result<Vec<RebasePlanEntry>> {
    let invalid = |message: &str| Error::InvalidInput(message.into());
    let size = std::fs::metadata(file).map_err(|source| Error::Io { action: "read the rebase plan file", source })?.len();
    if size > 4 * 1024 * 1024 {
        return Err(invalid("The plan file is larger than 4 MiB."));
    }
    let bytes = std::fs::read(file).map_err(|source| Error::Io { action: "read the rebase plan file", source })?;
    let rows: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("The plan file must be a JSON array of rows."))?;
    rows.as_array().ok_or_else(|| invalid("The plan file must be a JSON array of rows."))?.iter().map(|row| {
        let row = row.as_object().ok_or_else(|| invalid("Each plan row is an object."))?;
        if row.contains_key("exec") {
            return Err(invalid("Plan rows that run commands (exec) must be reviewed in Rocket's desktop UI."));
        }
        if row.keys().any(|k| !matches!(k.as_str(), "commit" | "instruction" | "message" | "stop_after")) {
            return Err(invalid("Plan rows take only commit, instruction, message and stop_after."));
        }
        let commit = row.get("commit").and_then(Value::as_str)
            .ok_or_else(|| invalid("Each plan row names its commit by full ID."))
            .and_then(|id| ObjectId::parse(&id.to_ascii_lowercase()))?;
        let instruction = match row.get("instruction").and_then(Value::as_str) {
            Some("pick") => RebaseInstruction::Pick,
            Some("reword") => RebaseInstruction::Reword,
            Some("edit") => RebaseInstruction::Edit,
            Some("squash") => RebaseInstruction::Squash,
            Some("fixup") => RebaseInstruction::Fixup,
            Some("drop") => RebaseInstruction::Drop,
            _ => return Err(invalid("instruction is pick, reword, edit, squash, fixup or drop.")),
        };
        let message = match row.get("message") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            _ => return Err(invalid("message is text.")),
        };
        let stop_after = match row.get("stop_after") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(stop)) => *stop,
            _ => return Err(invalid("stop_after is true or false.")),
        };
        Ok(RebasePlanEntry { commit, instruction, message, exec: None, stop_after })
    }).collect()
}
