//! Bisect through the desktop's state machine. A decision names the checkout it
//! was tested on. An automatic run pins the reviewed test executable's SHA-256 and
//! holds repository write ownership for the whole search, as the desktop does;
//! results are saved where the desktop's Bisect page reads them.
use crate::{
    advanced::review_only,
    args::Command,
    output::{self as out, Outcome, Problem},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::BisectStatus => {
            let session = repo.bisect_session(cancel)?;
            let mut o = Outcome::success(json!({"active":session.is_some(),"session":session.as_ref().map(session_json),"tests":null}));
            if session.is_some() {
                o.data["tests"] = o.observe("bisect_tests", &repo.bisect_test_history(cancel), |t| json!(t.iter().map(test).collect::<Vec<_>>()));
            }
            Ok(o)
        }
        Command::Bisect { action, good, bad, expected, preview } => {
            let p = repo.preview_bisect(*action, good, bad, cancel)?;
            if let Some(expected) = expected
                && p.session().and_then(|s| target(&s.current)) != Some(expected.as_str())
            {
                return Err(Error::Bisect(BisectFailure::StalePreview));
            }
            let review = json!({"action":action.label(),"good":p.good().as_str(),"bad":p.bad().as_str(),
                "original":out::head(p.original()),"candidates":p.candidates(),"session":p.session().map(session_json)});
            if *preview {
                return Ok(review_only(review));
            }
            let r = repo.apply_bisect(&p, cancel)?;
            let mut o = Outcome::success(json!({"preview":review,"command_result":out::observation(&r.result,out::write_output)}));
            o.primary(&r.result);
            o.data["session"] = o.observe("bisect_session", &r.session, |s| s.as_ref().map_or(Value::Null, session_json));
            o.data["status"] = o.observe("status", &r.status, out::status);
            o.data["operation"] = o.observe("operation", &r.state, out::operation);
            if let Some(f) = r.failure {
                o.data["bisect_failure"] = json!({"code":f.code(),"guidance":f.guidance()});
                if o.error.as_ref().is_some_and(|e| e.exit != 130 && e.exit != 124) {
                    o.error = Some(Problem::new(f.code(), "Bisect needs attention.", f.guidance(), 1));
                }
            }
            Ok(o)
        }
        Command::BisectRun { range, program, arguments, timeout, expected, preview } => {
            // Without this check a GOOD BAD range would silently continue an older search.
            match (range.is_some(), repo.bisect_session(cancel)?.is_some()) {
                (true, true) => return Ok(Outcome::failure(Problem::new("bisect_active_operation", "A bisect is already active.",
                    "Continue it with bisect run --continue, or end it with bisect restore.", 1))),
                (false, false) => return Ok(Outcome::failure(Problem::new("bisect_not_active", "No Rocket bisect is active.",
                    "Start one with bisect run GOOD BAD or bisect start GOOD BAD.", 1))),
                _ => {}
            }
            let command = BisectTestCommand { program: program.clone(), arguments: arguments.clone(), timeout_seconds: *timeout };
            let (good, bad) = range.as_ref().map_or(("", ""), |(g, b)| (g.as_str(), b.as_str()));
            let p = repo.preview_bisect_auto(good, bad, &command, cancel)?;
            let step = p.step();
            let review = json!({"continue":range.is_none(),"good":step.good().as_str(),"bad":step.bad().as_str(),
                "original":out::head(step.original()),"candidates":step.candidates(),
                "program":{"path":out::os_path(program),"sha256":p.program_digest()},"arguments":arguments,"test_timeout_secs":timeout});
            if *preview {
                return Ok(review_only(review));
            }
            if expected.as_deref() != Some(p.program_digest()) {
                return Ok(Outcome::failure(Problem::new("bisect_program_changed", "The test executable differs from the reviewed one.",
                    "Run bisect run --preview again and review the executable before running it. Nothing was started.", 1)));
            }
            let r = repo.run_bisect_auto(&p, &output_folder()?, None, cancel)?;
            let phase = r.session.as_ref().ok().and_then(Option::as_ref).map(|s| s.phase);
            let result = match phase {
                Some(BisectPhase::Found) => "found",
                Some(BisectPhase::Inconclusive) => "inconclusive",
                _ => "stopped",
            };
            let mut o = Outcome::success(json!({"preview":review,"result":result,"tests":r.tests.iter().map(test).collect::<Vec<_>>()}));
            o.primary(&r.result);
            // A test's own time limit or failure is not a Git limit or an unknown error.
            match (r.result.as_ref().err(), r.tests.last().map(|t| t.outcome)) {
                (Some(Error::TimedOut), Some(BisectTestOutcome::TimedOut)) => o.error = Some(Problem::new("bisect_test_timed_out",
                    "The test exceeded --test-timeout.",
                    "That revision was not classified and the search is still active. Raise --test-timeout or fix the test, then run bisect run --continue, or end the search with bisect restore.", 124)),
                (Some(Error::InvalidInput(_) | Error::OutputLimit { .. }), Some(BisectTestOutcome::Failed)) => o.error = Some(Problem::new("bisect_test_failed",
                    "The test ended without a good, bad or skip result.",
                    "Exit 0 marks good, 1-124 bad and 125 skip; other exits and output beyond the limit stop without classifying. Fix the test, then run bisect run --continue, or end the search with bisect restore.", 1)),
                _ => {}
            }
            if result == "inconclusive" {
                o.warnings.push(json!({"code":"bisect_inconclusive","message":"Skipped revisions leave more than one possible first bad commit; see session.candidate_ids."}));
            }
            o.data["session"] = o.observe("bisect_session", &r.session, |s| s.as_ref().map_or(Value::Null, session_json));
            o.data["status"] = o.observe("status", &r.status, out::status);
            o.data["operation"] = o.observe("operation", &r.state, out::operation);
            Ok(o)
        }
        _ => unreachable!("only bisect commands are dispatched here"),
    }
}

/// Test output reaches private files here while a test runs; each is deleted after it.
fn output_folder() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("Rocket").join("cli").join("bisect-output"))
        .ok_or_else(|| Error::InvalidInput("LOCALAPPDATA must name an absolute folder for bisect test output.".into()))
}
fn target(head: &Head) -> Option<&str> {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => Some(target.as_str()),
        Head::Unborn { .. } => None,
    }
}
fn session_json(s: &BisectSession) -> Value {
    let settled = matches!(s.phase, BisectPhase::Found | BisectPhase::Inconclusive);
    json!({"phase":match s.phase {BisectPhase::Testing=>"testing",BisectPhase::CheckoutPending=>"checkout_pending",
            BisectPhase::Found=>"found",BisectPhase::Inconclusive=>"inconclusive",BisectPhase::RestoreOnly=>"restore_only"},
        "original":out::head(&s.original),"current":out::head(&s.current),"good":s.good.as_str(),"bad":s.bad.as_str(),
        "target":s.target.as_ref().map(ObjectId::as_str),
        "first_bad":(s.phase==BisectPhase::Found).then(||s.target.as_ref().map(ObjectId::as_str)).flatten(),
        "candidates":s.candidates.len(),
        "candidate_ids":settled.then(||s.candidates.iter().map(ObjectId::as_str).collect::<Vec<_>>()),
        "decisions":s.decisions.iter().map(|d|json!({"action":decision(d.action),"commit":d.commit.as_str()})).collect::<Vec<_>>(),
        "recovery_reference":s.recovery_reference})
}
fn decision(action: BisectAction) -> &'static str {
    match action {
        BisectAction::Good => "good",
        BisectAction::Bad => "bad",
        BisectAction::Skip => "skip",
        other => other.label(),
    }
}
fn test(t: &BisectTestResult) -> Value {
    let outcome = match t.outcome {
        BisectTestOutcome::Running => "running",
        BisectTestOutcome::Good => "good",
        BisectTestOutcome::Bad => "bad",
        BisectTestOutcome::Skip => "skip",
        BisectTestOutcome::Cancelled => "cancelled",
        BisectTestOutcome::TimedOut => "timeout",
        BisectTestOutcome::Failed => "failed",
        BisectTestOutcome::StateChanged => "changed",
    };
    json!({"commit":t.commit.as_str(),"outcome":outcome,"exit_code":t.exit_code,"elapsed_ms":t.elapsed_ms,"recorded":t.recorded,
        "stdout":out::bytes(&t.stdout),"stderr":out::bytes(&t.stderr),"truncated":t.truncated})
}
