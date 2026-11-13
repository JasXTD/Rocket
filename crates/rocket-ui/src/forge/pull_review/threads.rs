use super::*;

fn action_label(action: ForgeThreadAction) -> &'static str {
    match action {
        ForgeThreadAction::Reply => tr("Reply"),
        ForgeThreadAction::Resolve => tr("Resolve"),
        ForgeThreadAction::Unresolve => tr("Reopen"),
    }
}

#[derive(Default)]
pub struct State {
    pub snapshot: Option<Arc<ForgePullThreads>>,
    pub selected: String,
    pub body: String,
    pub action: Option<ForgeThreadAction>,
    pub preview: Option<Arc<ForgeThreadPreview>>,
    pub unknown: bool,
    pub checked: bool,
}
pub(super) fn start(
    s: &mut super::super::State,
    backend: &crate::backend::Backend,
    session: Arc<ForgeSession>,
    action: &str,
) -> rocket_git::Result<WriteTask<Done>> {
    let backend = backend.clone();
    let repo = s
        .pull
        .repository
        .clone()
        .ok_or_else(|| Error::InvalidInput(tr("Select a PR first.").into()))?;
    let number = s.pull.number;
    match action {
        "pull-threads-refresh" => {
            s.pull.threads.preview = None;
            s.pull.threads.checked = false;
            s.operation = "pull-threads-refresh";
            WriteTask::spawn(move |c| {
                backend
                    .forge_request("hosting-thread-details", &session, |session| {
                        session.pull_threads(&repo, number, &c)
                    })
                    .map(Done::PullThreads)
            })
        }
        "pull-thread-preview" => {
            if s.pull.threads.unknown || s.pull.unknown.is_some() || s.pull.merge.unknown.is_some()
            {
                return Err(Error::InvalidInput(
                    tr("Check the previous remote outcome first.").into(),
                ));
            }
            let snapshot = s
                .pull
                .threads
                .snapshot
                .clone()
                .ok_or_else(|| Error::InvalidInput(tr("Load discussion threads first.").into()))?;
            let index=snapshot.threads.iter().position(|t|t.id==s.pull.threads.selected).ok_or_else(||Error::InvalidInput(tr("Select a loaded thread. Saved text is preserved if its thread is no longer returned.").into()))?;
            let action =
                s.pull.threads.action.ok_or_else(|| {
                    Error::InvalidInput(tr("Choose Reply, Resolve or Reopen.").into())
                })?;
            let body = s.pull.threads.body.clone();
            s.pull.threads.preview = None;
            s.operation = "pull-thread-preview";
            WriteTask::spawn(move |c| {
                backend
                    .forge_request("hosting-thread-preview", &session, |session| {
                        session.preview_pull_thread(&snapshot, index, action, &body, &c)
                    })
                    .map(Done::PullThreadPreview)
            })
        }
        "pull-thread-submit" => {
            if s.pull.threads.unknown || s.pull.unknown.is_some() || s.pull.merge.unknown.is_some()
            {
                return Err(Error::InvalidInput(
                    tr("Check the previous remote outcome first.").into(),
                ));
            }
            let preview = s
                .pull
                .threads
                .preview
                .clone()
                .ok_or_else(|| Error::InvalidInput(tr("Review the thread action first.").into()))?;
            s.operation = "pull-thread-submit";
            WriteTask::spawn(move |c| {
                backend
                    .forge_request("hosting-thread-submit", &session, |session| {
                        session.submit_pull_thread(&preview, &c)
                    })
                    .map(|_| Done::PullThreadSubmitted)
            })
        }
        _ => Err(Error::InvalidInput(tr("Unknown discussion action.").into())),
    }
}
pub(super) fn section(
    ui: &mut Ui,
    s: &mut super::super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    super::card(ui, |ui, header| {
        if header {
            ui.label(RichText::new(tr("Review threads")).font(semibold(13.0)));
            let loaded = s.pull.threads.snapshot.as_ref().map(|snapshot| snapshot.threads.len());
            if let Some(n) = loaded.filter(|n| *n > 0) {
                ui.label(RichText::new(n.to_string()).size(12.5).color(FAINT()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if loaded.is_some() { tr("Refresh") } else { tr("Load threads") };
                if btn(ui, widgets, "hosting-threads-refresh", label, !busy) {
                    out.action = Some("pull-threads-refresh");
                }
            });
            return;
        }
        body(ui, &mut s.pull.threads, widgets, busy, out);
    });
}
fn body(
    ui: &mut Ui,
    t: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    if t.unknown {
        ui.add(Label::new(RichText::new(tr("Thread action outcome unknown. Refresh the threads and inspect this PR on the server before deciding to send again.")).size(12.5).color(AMBER())).wrap());
        if btn(
            ui,
            widgets,
            "hosting-thread-acknowledge",
            tr("I checked this thread; allow a new preview"),
            !busy && t.checked,
        ) {
            t.unknown = false;
            t.checked = false;
            t.preview = None;
        }
    }
    if let Some(preview) = t.preview.clone() {
        pages::well(ui, |ui| {
            ui.label(RichText::new(format!("{} · {}", action_label(preview.action()), preview.thread().path)).strong());
            ui.label(RichText::new(trf!("PR #{number} · {commit}", number = preview.number(), commit = &preview.head()[..12])).size(12.0).color(FAINT()));
            if !preview.body().is_empty() {
                ui.add(Label::new(preview.body()).wrap());
            }
            ui.add(Label::new(RichText::new(tr("This changes a remote discussion and may notify participants. Rocket rechecks the thread and commit; the server can still change after that check. Preview expires in two minutes.")).size(12.5).color(AMBER())).wrap());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = pages::primary(ui, !busy && !t.unknown, tr("Send reviewed action"));
                control(widgets, "hosting-thread-submit", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("pull-thread-submit");
                }
                if btn(ui, widgets, "hosting-thread-back", tr("Back to draft"), !busy) {
                    t.preview = None;
                }
            });
        });
        return;
    }
    let Some(snapshot) = t.snapshot.clone() else {
        pages::note(ui, tr("Load threads to reply, resolve or reopen."));
        // A reply saved before a restart keeps its text, even before threads load.
        if let Some(action) = t.action {
            composer(ui, t, widgets, busy, out, action);
        }
        return;
    };
    if snapshot.limited {
        ui.add(Label::new(RichText::new(tr("This is a bounded discussion preview. Open the provider for the complete discussion.")).size(12.5).color(AMBER())).wrap());
    }
    if snapshot.threads.is_empty() {
        pages::note(ui, tr("No review threads were found in this preview."));
    }
    if snapshot.repository().config().provider() == ForgeProvider::Gitea {
        pages::note(ui, tr("Gitea supports resolving and reopening review comments. Replies require the provider website with the stable API."));
    }
    for (i, thread) in snapshot.threads.iter().enumerate() {
        if i > 0 {
            pages::divider(ui);
        }
        // Where: the file and lines, then whether it is resolved.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let location = match (thread.start.or(thread.line), thread.line) {
                (Some(start), Some(end)) if start != end => format!("{start}–{end}"),
                (_, Some(line)) => line.to_string(),
                _ => tr("original location").into(),
            };
            let path = if thread.path.is_empty() { tr("General discussion") } else { &thread.path };
            ui.label(RichText::new(path).monospace().size(12.5));
            ui.label(RichText::new(format!("· {location}{}", if thread.outdated { tr(" · Outdated") } else { "" })).size(12.0).color(FAINT()));
            let (state, color) = if thread.resolved { (tr("Resolved"), GREEN()) } else { (tr("Unresolved"), AMBER()) };
            pages::pill(ui, state, color);
        });
        for c in &thread.comments {
            // Replies sit under the comment that started the thread.
            ui.horizontal(|ui| {
                if c.reply_to.is_some() {
                    ui.add_space(16.0);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(&c.author).strong());
                        ui.label(RichText::new(&c.date).size(12.0).color(FAINT()));
                    });
                    ui.add(Label::new(&c.body).wrap());
                });
            });
        }
        if thread.limited {
            pages::note(ui, tr("This thread is incomplete; use the provider website to reply or change its resolution."));
        }
        // Reply always shows (disabled where a host or thread can't take one);
        // Resolve or Reopen shows for the thread's current state.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let can_select = t.body.is_empty() || t.selected == thread.id;
            for (key, action, shown, enabled) in [
                ("reply", ForgeThreadAction::Reply, true, thread.can_reply),
                ("resolve", ForgeThreadAction::Resolve, !thread.resolved, thread.can_resolve),
                ("unresolve", ForgeThreadAction::Unresolve, thread.resolved, thread.can_unresolve),
            ] {
                if shown
                    && btn(
                        ui,
                        widgets,
                        &format!("hosting-thread-{key}-{i}"),
                        action_label(action),
                        !busy
                            && !t.unknown
                            && can_select
                            && enabled
                            && !thread.limited
                            && (t.body.is_empty() || action == ForgeThreadAction::Reply),
                    )
                {
                    t.selected = thread.id.clone();
                    t.action = Some(action);
                }
            }
        });
        if t.selected == thread.id
            && let Some(action) = t.action
        {
            composer(ui, t, widgets, busy, out, action);
        }
    }
    // A saved reply whose thread is no longer returned keeps its text here.
    if let Some(action) = t.action
        && !snapshot.threads.iter().any(|thread| thread.id == t.selected)
    {
        pages::divider(ui);
        composer(ui, t, widgets, busy, out, action);
    }
}
/// The chosen action for one thread: the reply text, or just the confirmation.
fn composer(
    ui: &mut Ui,
    t: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
    action: ForgeThreadAction,
) {
    pages::well(ui, |ui| {
        ui.label(RichText::new(action_label(action)).strong());
        if action == ForgeThreadAction::Reply {
            let r = ui.add_enabled(
                !busy,
                TextEdit::multiline(&mut t.body)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .char_limit(8192)
                    .hint_text(tr("Reply saved locally until submitted")),
            );
            control(widgets, "hosting-thread-body", &r, ui.clip_rect());
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = pages::primary(ui, !busy && !t.unknown, tr("Review before sending"));
            control(widgets, "hosting-thread-preview", &r, ui.clip_rect());
            if r.clicked() {
                out.action = Some("pull-thread-preview");
            }
            if btn(ui, widgets, "hosting-thread-discard", tr("Discard"), !busy && !t.unknown) {
                t.body.clear();
                t.action = None;
                t.selected.clear();
            }
        });
    });
}
