use super::*;
use crate::i18n::mark;

// The API layer owns these warnings. Register its current static guidance here,
// while leaving an unfamiliar future warning intact instead of substituting it.
const MERGE_GUIDANCE: &[&str] = &[
    mark("The request includes the reviewed head. The project's configured merge method still applies; Rocket does not override it. Target tips can change after Rocket's check, and GitLab enforces its merge rules."),
    mark("Bitbucket Cloud does not document an expected-head condition for this endpoint. Both tips are rechecked immediately before sending, but either can change afterward. Inspect the final PR before relying on the result."),
    mark("The server checks the reviewed PR version. Branch tips are rechecked before sending, but can change afterward. Server merge checks remain authoritative."),
    mark("The request includes the reviewed source commit. Azure may finish asynchronously; an unconfirmed result requires a fresh PR inspection. Target policy is enforced by Azure."),
    mark("The request includes the reviewed head condition. Rocket also rechecks the target, which can still change after that check. The provider enforces its merge rules."),
];

fn method_label(provider: ForgeProvider, method: ForgeMergeMethod) -> &'static str {
    if provider == ForgeProvider::GitLab && method == ForgeMergeMethod::Merge {
        tr("Merge with project policy")
    } else {
        match method {
            ForgeMergeMethod::Merge => tr("Merge commit"),
            ForgeMergeMethod::Squash => tr("Squash and merge"),
            ForgeMergeMethod::Rebase => tr("Rebase and merge"),
        }
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
            self.result = Some(tr("The provider now reports this PR merged. The refresh cannot identify which submission merged it.").into());
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
            tr("Check the previous submission's outcome before merging.").into(),
        ));
    }
    if s.pull.has_draft() || s.pull.preview.is_some() {
        return Err(Error::InvalidInput(
            tr("Submit or discard the review draft before merging.").into(),
        ));
    }
    let backend = backend.clone();
    if action == "pull-merge-preview" {
        let details = s
            .pull
            .details
            .clone()
            .ok_or_else(|| Error::InvalidInput(tr("Load PR details first.").into()))?;
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
            .ok_or_else(|| Error::InvalidInput(tr("Review the merge first.").into()))?;
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
        ui.colored_label(AMBER(), trf!("Merge outcome unknown for {repository} #{number}. The request may have completed on the provider. Refresh that PR and inspect the provider before another attempt.", repository = repo.path(), number));
        if btn(
            ui,
            widgets,
            "hosting-merge-acknowledge",
            tr("I checked this PR on the provider; allow a new merge preview"),
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
    let open = !details.summary().merged && details.summary().state == "open";
    // A closed PR has nothing left to choose; the header's pill already says why.
    if !open && s.pull.merge.preview.is_none() && s.pull.merge.result.is_none() {
        return;
    }
    let provider = details.repository().config().provider();
    let allowed = !busy
        && s.pull.unknown.is_none()
        && s.pull.merge.unknown.is_none()
        && !s.pull.threads.unknown
        && !s.pull.has_draft()
        && s.pull.preview.is_none();
    super::card(ui, |ui, header| {
        if header {
            ui.label(RichText::new(tr("Merge")).font(semibold(13.0)));
            if let Some(preview) = &s.pull.merge.preview {
                ui.label(RichText::new(method_label(provider, preview.method())).size(12.5).color(MUTED()));
            }
            return;
        }
        if let Some(result) = &s.pull.merge.result {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if matches!(result.len(), 40 | 64) && result.bytes().all(|b| b.is_ascii_hexdigit()) {
                    pages::pill(ui, tr("Merged"), VIOLET());
                    super::chip(ui, &result[..12], TEXT()).on_hover_text(result);
                } else {
                    ui.add(Label::new(RichText::new(result).color(GREEN())).wrap());
                }
            });
        }
        if let Some(preview) = s.pull.merge.preview.clone() {
            // Exactly what will be sent: where to, and the two commits reviewed.
            ui.label(format!("{} #{} → {}", preview.repository().path(), preview.summary().number, preview.summary().target));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (head, base) = (&preview.summary().head, &preview.summary().base);
                super::chip(ui, &trf!("head {commit}", commit = &head[..12.min(head.len())]), TEXT()).on_hover_text(head);
                super::chip(ui, &trf!("base {commit}", commit = &base[..12.min(base.len())]), TEXT()).on_hover_text(base);
            });
            let condition = details.merge_condition();
            let guidance = MERGE_GUIDANCE.iter().find(|&&known| known == condition).map_or(condition, |&known| tr(known));
            ui.add(Label::new(RichText::new(guidance).size(12.5).color(AMBER())).wrap());
            pages::note(ui, tr("This updates the remote target branch. Local files stay unchanged and no branch is deleted."));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = pages::primary(ui, allowed, tr("Merge reviewed PR"));
                control(widgets, "hosting-merge-submit", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("pull-merge-submit");
                }
                if btn(ui, widgets, "hosting-merge-back", tr("Cancel preview"), !busy) {
                    s.pull.merge.preview = None;
                }
            });
        } else if open {
            // The methods this host can merge with, and Review merge, on one row.
            let methods: Vec<_> = [ForgeMergeMethod::Merge, ForgeMergeMethod::Squash, ForgeMergeMethod::Rebase]
                .into_iter()
                .filter(|m| details.supports_merge(*m))
                .collect();
            // A method picked on another host's PR may not be offered here (only some
            // hosts rebase); start from the first one this host offers.
            if let Some(&first) = methods.first()
                && !methods.contains(&s.pull.merge.method)
            {
                s.pull.merge.method = first;
            }
            ui.horizontal(|ui| {
                let items: Vec<_> = methods.iter().map(|m| (method_label(provider, *m), s.pull.merge.method == *m)).collect();
                let responses = ui.add_enabled_ui(allowed, |ui| pages::segments(ui, &items)).inner;
                for (method, r) in methods.iter().zip(&responses) {
                    control(widgets, format!("hosting-merge-method-{}", method.key()), r, ui.clip_rect());
                    if r.clicked() {
                        s.pull.merge.method = *method;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = pages::primary(ui, allowed && methods.contains(&s.pull.merge.method), tr("Review merge"));
                    control(widgets, "hosting-merge-preview", &r, ui.clip_rect());
                    if r.clicked() {
                        out.action = Some("pull-merge-preview");
                    }
                });
            });
            if s.pull.has_draft() || s.pull.preview.is_some() {
                ui.add(Label::new(RichText::new(tr("Submit or discard the review draft before merging.")).size(12.5).color(AMBER())).wrap());
            }
        }
    });
}
