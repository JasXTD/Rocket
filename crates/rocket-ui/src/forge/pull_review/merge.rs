use super::*;

fn method_label(provider: ForgeProvider, method: ForgeMergeMethod) -> &'static str {
    if provider == ForgeProvider::GitLab && method == ForgeMergeMethod::Merge {
        "Merge with project policy"
    } else {
        method.label()
    }
}

#[derive(Default)]
pub struct State {
    pub preview: Option<Arc<ForgePullMergePreview>>,
    pub method: ForgeMergeMethod,
    pub result: Option<String>,
    pub unknown: Option<(ForgeRepository, u64)>,
    pub checked_unknown: bool,
    pub submission: Option<(ForgeRepository, u64)>,
}
impl State {
    pub fn reconcile(&mut self, details: &ForgePullDetails) {
        self.checked_unknown = self.unknown.as_ref().is_some_and(|(repo, number)| {
            repo == details.repository() && *number == details.summary().number
        });
        if self.checked_unknown && details.summary().merged {
            self.unknown = None;
            self.checked_unknown = false;
            self.result = Some("The provider now reports this PR merged. The refresh cannot identify which submission merged it.".into());
        }
    }
}
pub(super) fn start(
    s: &mut super::super::State,
    backend: &crate::backend::Backend,
    session: Arc<ForgeSession>,
    action: &str,
) -> rocket_git::Result<WriteTask<Done>> {
    if s.pull.merge.unknown.is_some() || s.pull.unknown.is_some() || s.pull.threads.unknown {
        return Err(Error::InvalidInput(
            "Check the previous submission's outcome before merging.".into(),
        ));
    }
    if s.pull.has_draft() || s.pull.preview.is_some() {
        return Err(Error::InvalidInput(
            "Submit or discard the review draft before merging.".into(),
        ));
    }
    let backend = backend.clone();
    if action == "pull-merge-preview" {
        let details = s
            .pull
            .details
            .clone()
            .ok_or_else(|| Error::InvalidInput("Load PR details first.".into()))?;
        let method = s.pull.merge.method;
        s.pull.merge.preview = None;
        s.pull.merge.result = None;
        s.operation = "pull-merge-preview";
        WriteTask::spawn(move |cancel| {
            backend
                .forge_request("hosting-merge-preview", &session, |session| {
                    session.preview_pull_merge(&details, method, &cancel)
                })
                .map(Done::PullMergePreview)
        })
    } else {
        let preview = s
            .pull
            .merge
            .preview
            .clone()
            .ok_or_else(|| Error::InvalidInput("Review the merge first.".into()))?;
        s.pull.merge.submission = Some((preview.repository().clone(), preview.summary().number));
        s.operation = "pull-merge-submit";
        WriteTask::spawn(move |cancel| {
            backend
                .forge_request("hosting-merge-submit", &session, |session| {
                    session.merge_pull(&preview, &cancel)
                })
                .map(Done::PullMerged)
        })
    }
}
pub(super) fn recovery(
    ui: &mut Ui,
    s: &mut super::super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
) {
    if let Some((repo, number)) = &s.pull.merge.unknown {
        ui.colored_label(AMBER(), format!("Merge outcome unknown for {} #{number}. The request may have completed on the provider. Refresh that PR and inspect the provider before another attempt.", repo.path()));
        if btn(
            ui,
            widgets,
            "hosting-merge-acknowledge",
            "I checked this PR on the provider; allow a new merge preview",
            !busy && s.pull.merge.checked_unknown,
        ) {
            s.pull.merge.unknown = None;
            s.pull.merge.checked_unknown = false;
            s.pull.merge.preview = None;
        }
    }
}
pub(super) fn section(
    ui: &mut Ui,
    s: &mut super::super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    let Some(details) = s.pull.details.clone() else {
        return;
    };
    let allowed = !busy
        && s.pull.unknown.is_none()
        && s.pull.merge.unknown.is_none()
        && !s.pull.threads.unknown
        && !s.pull.has_draft()
        && s.pull.preview.is_none();
    ui.separator();
    ui.strong("Merge on provider");
    if let Some(result) = &s.pull.merge.result {
        ui.add(Label::new(RichText::new(result).color(GREEN())).wrap());
    }
    if let Some(preview) = s.pull.merge.preview.clone() {
        ui.label(format!(
            "{} · {} #{} → {}",
            method_label(details.repository().config().provider(), preview.method()),
            preview.repository().path(),
            preview.summary().number,
            preview.summary().target
        ));
        ui.add(
            Label::new(
                RichText::new(format!(
                    "Reviewed head {}\nReviewed base {}",
                    preview.summary().head,
                    preview.summary().base
                ))
                .monospace(),
            )
            .wrap(),
        );
        ui.colored_label(AMBER(), details.merge_condition());
        ui.label("This updates the remote target branch. Local files stay unchanged and no branch is deleted.");
        ui.horizontal_wrapped(|ui| {
            if btn(ui, widgets, "hosting-merge-back", "Cancel preview", !busy) {
                s.pull.merge.preview = None;
            }
            if btn(
                ui,
                widgets,
                "hosting-merge-submit",
                "Merge reviewed PR",
                allowed,
            ) {
                out.action = Some("pull-merge-submit");
            }
        });
    } else if details.summary().merged || details.summary().state != "open" {
        ui.label("This PR is no longer open for merging.");
    } else {
        ui.horizontal_wrapped(|ui| {
            for method in [
                ForgeMergeMethod::Merge,
                ForgeMergeMethod::Squash,
                ForgeMergeMethod::Rebase,
            ] {
                let r = ui.add_enabled(
                    allowed && details.supports_merge(method),
                    Button::selectable(s.pull.merge.method == method, method_label(details.repository().config().provider(), method)),
                );
                control(
                    widgets,
                    format!("hosting-merge-method-{}", method.key()),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    s.pull.merge.method = method;
                }
            }
            if btn(
                ui,
                widgets,
                "hosting-merge-preview",
                "Review merge",
                allowed,
            ) {
                out.action = Some("pull-merge-preview");
            }
        });
        ui.colored_label(MUTED(), "Preview rechecks the PR and available readiness data. The server enforces permissions and policy, and may queue a merge. Rocket does not bypass policy or automatically retry requests.");
        if s.pull.has_draft() {
            ui.colored_label(AMBER(), "Submit or discard your review draft before merging.");
        }
    }
}
