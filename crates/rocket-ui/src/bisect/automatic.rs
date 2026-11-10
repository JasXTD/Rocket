use super::*;
use std::{path::PathBuf, sync::Mutex};

pub struct State {
    pub enabled: bool,
    pub program: String,
    pub arguments: String,
    pub seconds: String,
    pub preview: Option<Arc<BisectAutoPreview>>,
    pub progress: Arc<Mutex<BisectAutoProgress>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            program: String::new(),
            arguments: String::new(),
            seconds: "60".into(),
            preview: None,
            progress: Default::default(),
        }
    }
}
impl State {
    pub fn command(&self) -> rocket_git::Result<BisectTestCommand> {
        let command = BisectTestCommand {
            program: PathBuf::from(self.program.trim()),
            arguments: if self.arguments.is_empty() {
                Vec::new()
            } else {
                self.arguments.lines().map(str::to_owned).collect()
            },
            timeout_seconds: self.seconds.parse().map_err(|_| {
                rocket_git::Error::InvalidInput(tr("Enter a timeout from 1 to 3,600 seconds.").into())
            })?,
        };
        command.validate()?;
        Ok(command)
    }
    pub fn fields(
        &mut self,
        ui: &mut Ui,
        widgets: &mut std::collections::HashMap<String, crate::app::UiControl>,
        waiting: bool,
    ) -> bool {
        // Widget map type is supplied by the shared page controls.
        let mut changed = false;
        let r = ui.add_enabled(
            !waiting,
            Checkbox::new(&mut self.enabled, tr("Run a test command automatically")),
        );
        pages::control(widgets, "dialog-bisect-auto", &r, ui.clip_rect());
        changed |= r.changed();
        if !self.enabled {
            return changed;
        }
        pages::caption(ui, tr("Test command"));
        for (value, label, key) in [
            (
                &mut self.program,
                tr("Executable · local absolute path"),
                "dialog-bisect-program",
            ),
            (
                &mut self.seconds,
                tr("Timeout per revision · 1–3,600 seconds"),
                "dialog-bisect-timeout",
            ),
        ] {
            ui.label(label);
            let r = ui.add_enabled(
                !waiting,
                TextEdit::singleline(value)
                    .desired_width(f32::INFINITY)
                    .char_limit(32768),
            );
            pages::control(widgets, key, &r, ui.clip_rect());
            changed |= r.changed();
        }
        ui.label(tr("Arguments · one literal argument per line"));
        let r = ui.add_enabled(
            !waiting,
            TextEdit::multiline(&mut self.arguments)
                .font(TextStyle::Monospace)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .char_limit(16000),
        );
        pages::control(widgets, "dialog-bisect-arguments", &r, ui.clip_rect());
        changed |= r.changed();
        pages::note(
            ui,
            tr("0 = Good · 1–124 = Bad · 125 = Skip · other codes or timeout stop without a decision."),
        );
        pages::note(
            ui,
            tr("The test runs with your account in each checked-out revision. Keep it outside this folder; Rocket stops on tracked edits."),
        );
        if let Some(p) = &self.preview {
            pages::caption(ui, tr("Reviewed automated search"));
            pages::well(ui, |ui| {
                pages::field(
                    ui,
                    tr("Working folder"),
                    // Without the \\?\ prefix of a verbatim path.
                    p.worktree().display().to_string().trim_start_matches(r"\\?\"),
                    false,
                );
                pages::field(
                    ui,
                    tr("Program"),
                    &custom_command_display(&p.command().program.to_string_lossy()),
                    true,
                );
                for (i, arg) in p.command().arguments.iter().enumerate() {
                    pages::field(
                        ui,
                        &trf!("Argument {number}", number = i + 1),
                        &format!("{:?}", arg),
                        true,
                    );
                }
                pages::field(ui, tr("Known good"), p.step().good().as_str().get(..12).unwrap_or(p.step().good().as_str()), true);
                pages::field(ui, tr("Known bad"), p.step().bad().as_str().get(..12).unwrap_or(p.step().bad().as_str()), true);
                pages::field(
                    ui,
                    tr("Return checkout"),
                    &head_text(p.step().original()),
                    false,
                );
            });
            pages::callout(
                ui,
                BLUE(),
                tr("Runs once per candidate, up to 256 tests. The original checkout is kept until you choose Restore original."),
            );
        }
        changed
    }
    pub fn results(&self, ui: &mut Ui) {
        let progress = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if progress.results.is_empty() {
            return;
        }
        pages::caption(ui, tr("Test results"));
        if let Some(id) = &progress.running {
            pages::waiting(ui, &trf!("Testing {commit}…", commit = &id.as_str()[..12]));
        }
        for (index, r) in progress.results.iter().enumerate().rev().take(30) {
            let label = trf!("{commit} · {outcome} · exit {code} · {elapsed_ms} ms{recorded}", commit = &r.commit.as_str()[..12], outcome = outcome_label(r.outcome), code = r.exit_code.map_or("—".into(), |v| v.to_string()), elapsed_ms = r.elapsed_ms, recorded = if r.recorded {
                    tr(" · recorded")
                } else {
                    tr(" · unclassified")
                });
            CollapsingHeader::new(label).id_salt(("bisect-test-output",index)).show(ui,|ui|{
                if r.stdout.is_empty()&&r.stderr.is_empty(){pages::note(ui,tr("No output retained in this window. Test summaries survive reopening; raw output is not written to diagnostics."));}
                for(key,label,bytes)in[("Standard output",tr("Standard output"),&r.stdout),("Standard error",tr("Standard error"),&r.stderr)]{if !bytes.is_empty(){ui.label(label);ScrollArea::both().id_salt((index,key)).max_height(150.).show(ui,|ui|{ui.monospace(custom_command_display(&String::from_utf8_lossy(bytes)));});}}
                if r.truncated{pages::note(ui,tr("Output truncated to the bounded preview."));}
            });
        }
        if progress.results.len() > 30 {
            pages::note(
                ui,
                tr("Showing the last 30 tests. All bounded summaries remain in the repository's bisect test record."),
            );
        }
    }
}

fn outcome_label(outcome: BisectTestOutcome) -> &'static str {
    match outcome {
        BisectTestOutcome::Running => tr("Running"),
        BisectTestOutcome::Good => tr("Good"),
        BisectTestOutcome::Bad => tr("Bad"),
        BisectTestOutcome::Skip => tr("Skip"),
        BisectTestOutcome::Cancelled => tr("Cancelled"),
        BisectTestOutcome::TimedOut => tr("Timed out"),
        BisectTestOutcome::Failed => tr("Command failed"),
        BisectTestOutcome::StateChanged => tr("Checkout changed"),
    }
}

pub(crate) fn outcome(out: &mut Outcome, r: BisectAutoReport) {
    out.command_succeeded =
        r.result.is_ok() && r.status.is_ok() && r.session.is_ok() && r.state.is_ok();
    out.recovery_pending = r.session.as_ref().is_ok_and(Option::is_some);
    out.summary=if out.command_succeeded {match r.session.as_ref().ok().and_then(|s|s.as_ref()).map(|s|s.phase){Some(BisectPhase::Found)=>tr("First bad commit found. Inspect the result, then Restore original."),Some(BisectPhase::Inconclusive)=>tr("Skipped tests leave several possible first bad commits. Review the candidates, then restore."),_=>tr("Automated tests finished. Refresh and inspect the bisect session.")}}else{tr("Automated bisect stopped. The current test may be unclassified; inspect the session before retrying or restoring.")}.into();
    if let Err(e) = r.result {
        out.details.push(user_error(&e));
    }
    if let Err(e) = r.status {
        out.details
            .push(trf!("Refresh changes: {error}", error = user_error(&e)));
    }
    if let Err(e) = r.session {
        out.details
            .push(trf!("Refresh bisect: {error}", error = user_error(&e)));
    }
    if let Err(e) = r.state {
        out.details
            .push(trf!("Refresh operation: {error}", error = user_error(&e)));
    }
    out.details.push(trn!(r.tests.len(), "{n} test summary retained. Generated files are not deleted; preserve edits before restoring.", "{n} test summaries retained. Generated files are not deleted; preserve edits before restoring."));
}
