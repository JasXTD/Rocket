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
    ui.separator();
    ui.strong(tr("Review threads"));
    if btn(
        ui,
        widgets,
        "hosting-threads-refresh",
        tr("Load / refresh threads"),
        !busy,
    ) {
        out.action = Some("pull-threads-refresh");
    }
    let t = &mut s.pull.threads;
    if t.unknown {
        ui.colored_label(AMBER(),tr("Thread action outcome unknown. Refresh the threads and inspect this PR on the server before deciding to send again."));
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
    if let Some(preview) = &t.preview {
        ui.strong(format!(
            "{} · {}",
            action_label(preview.action()),
            preview.thread().path
        ));
        ui.label(trf!("PR #{number} · {commit}", number = preview.number(), commit = &preview.head()[..12]));
        ui.add(Label::new(preview.body()).wrap());
        ui.colored_label(AMBER(),tr("This changes a remote discussion and may notify participants. Rocket rechecks the thread and commit; the server can still change after that check. Preview expires in two minutes."));
        ui.horizontal_wrapped(|ui| {
            if btn(ui, widgets, "hosting-thread-back", tr("Back to draft"), !busy) {
                t.preview = None;
            }
            if btn(
                ui,
                widgets,
                "hosting-thread-submit",
                tr("Send reviewed action"),
                !busy && !t.unknown,
            ) {
                out.action = Some("pull-thread-submit");
            }
        });
        return;
    }
    if let Some(snapshot) = &t.snapshot {
        if snapshot.limited {
            ui.colored_label(
                AMBER(),
                tr("This is a bounded discussion preview. Open the provider for the complete discussion."),
            );
        }
        if snapshot.threads.is_empty() {
            ui.colored_label(MUTED(), tr("No review threads were found in this preview."));
        }
        if snapshot.repository().config().provider() == ForgeProvider::Gitea {
            ui.colored_label(MUTED(), tr("Gitea supports resolving and reopening review comments. Replies require the provider website with the stable API."));
        }
        for (i, thread) in snapshot.threads.iter().enumerate() {
            ui.separator();
            let location = match (thread.start.or(thread.line), thread.line) {
                (Some(start), Some(end)) if start != end => format!("{start}–{end}"),
                (_, Some(line)) => line.to_string(),
                _ => tr("original location").into(),
            };
            ui.add(
                Label::new(format!(
                    "{} · {} · {}{}",
                    if thread.path.is_empty() { tr("General discussion") } else { &thread.path },
                    location,
                    if thread.resolved {
                        tr("Resolved")
                    } else {
                        tr("Unresolved")
                    },
                    if thread.outdated { tr(" · Outdated") } else { "" }
                ))
                .wrap(),
            );
            for c in &thread.comments {
                ui.strong(&c.author);
                ui.add(Label::new(&c.body).wrap());
            }
            if thread.limited {
                ui.colored_label(MUTED(), tr("This thread is incomplete; use the provider website to reply or change its resolution."));
            }
            ui.horizontal_wrapped(|ui| {
                let can_select = t.body.is_empty() || t.selected == thread.id;
                for (key, action, enabled) in [
                    (
                        "reply",
                        ForgeThreadAction::Reply,
                        thread.can_reply,
                    ),
                    (
                        "resolve",
                        ForgeThreadAction::Resolve,
                        !thread.resolved && thread.can_resolve,
                    ),
                    (
                        "unresolve",
                        ForgeThreadAction::Unresolve,
                        thread.resolved && thread.can_unresolve,
                    ),
                ] {
                    if btn(
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
                    ) {
                        t.selected = thread.id.clone();
                        t.action = Some(action);
                    }
                }
            });
        }
    } else {
        ui.colored_label(
            MUTED(),
            tr("Load threads to reply or change resolution. Loading does not modify the PR."),
        );
    }
    if let Some(action) = t.action {
        ui.separator();
        ui.strong(action_label(action));
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
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                widgets,
                "hosting-thread-preview",
                tr("Review thread action"),
                !busy && !t.unknown,
            ) {
                out.action = Some("pull-thread-preview");
            }
            if btn(
                ui,
                widgets,
                "hosting-thread-discard",
                tr("Discard reply draft"),
                !busy && !t.unknown,
            ) {
                t.body.clear();
                t.action = None;
                t.selected.clear();
            }
        });
    }
}
