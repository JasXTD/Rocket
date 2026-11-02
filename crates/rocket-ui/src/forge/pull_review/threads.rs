use super::*;

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
        .ok_or_else(|| Error::InvalidInput("Select a PR first.".into()))?;
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
                    "Check the previous remote outcome first.".into(),
                ));
            }
            let snapshot = s
                .pull
                .threads
                .snapshot
                .clone()
                .ok_or_else(|| Error::InvalidInput("Load discussion threads first.".into()))?;
            let index=snapshot.threads.iter().position(|t|t.id==s.pull.threads.selected).ok_or_else(||Error::InvalidInput("Select a loaded thread. Saved text is preserved if its thread is no longer returned.".into()))?;
            let action =
                s.pull.threads.action.ok_or_else(|| {
                    Error::InvalidInput("Choose Reply, Resolve or Reopen.".into())
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
                    "Check the previous remote outcome first.".into(),
                ));
            }
            let preview = s
                .pull
                .threads
                .preview
                .clone()
                .ok_or_else(|| Error::InvalidInput("Review the thread action first.".into()))?;
            s.operation = "pull-thread-submit";
            WriteTask::spawn(move |c| {
                backend
                    .forge_request("hosting-thread-submit", &session, |session| {
                        session.submit_pull_thread(&preview, &c)
                    })
                    .map(|_| Done::PullThreadSubmitted)
            })
        }
        _ => Err(Error::InvalidInput("Unknown discussion action.".into())),
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
    ui.strong("Review threads");
    if btn(
        ui,
        widgets,
        "hosting-threads-refresh",
        "Load / refresh threads",
        !busy,
    ) {
        out.action = Some("pull-threads-refresh");
    }
    let t = &mut s.pull.threads;
    if t.unknown {
        ui.colored_label(AMBER(),"Thread action outcome unknown. Refresh the threads and inspect this PR on the server before deciding to send again.");
        if btn(
            ui,
            widgets,
            "hosting-thread-acknowledge",
            "I checked this thread; allow a new preview",
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
            preview.action().label(),
            preview.thread().path
        ));
        ui.label(format!(
            "PR #{} · {}",
            preview.number(),
            &preview.head()[..12]
        ));
        ui.add(Label::new(preview.body()).wrap());
        ui.colored_label(AMBER(),"This changes a remote discussion and may notify participants. Rocket rechecks the thread and commit; the server can still change after that check. Preview expires in two minutes.");
        ui.horizontal_wrapped(|ui| {
            if btn(ui, widgets, "hosting-thread-back", "Back to draft", !busy) {
                t.preview = None;
            }
            if btn(
                ui,
                widgets,
                "hosting-thread-submit",
                "Send reviewed action",
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
                "First 50 threads shown. Open the provider for the complete discussion.",
            );
        }
        for (i, thread) in snapshot.threads.iter().enumerate() {
            ui.separator();
            let location = match (thread.start.or(thread.line), thread.line) {
                (Some(start), Some(end)) if start != end => format!("{start}–{end}"),
                (_, Some(line)) => line.to_string(),
                _ => "original location".into(),
            };
            ui.add(
                Label::new(format!(
                    "{} · {} · {}{}",
                    thread.path,
                    location,
                    if thread.resolved {
                        "Resolved"
                    } else {
                        "Unresolved"
                    },
                    if thread.outdated { " · Outdated" } else { "" }
                ))
                .wrap(),
            );
            for c in &thread.comments {
                ui.strong(&c.author);
                ui.add(Label::new(&c.body).wrap());
            }
            if thread.limited {
                ui.colored_label(MUTED(), "First 20 comments shown.");
            }
            ui.horizontal_wrapped(|ui| {
                let can_select = t.body.is_empty() || t.selected == thread.id;
                for (key, action, enabled) in [
                    (
                        "reply",
                        ForgeThreadAction::Reply,
                        !thread.comments.is_empty(),
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
                        action.label(),
                        !busy
                            && !t.unknown
                            && can_select
                            && enabled
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
            "Load threads to reply or change resolution. Loading does not modify the PR.",
        );
    }
    if let Some(action) = t.action {
        ui.separator();
        ui.strong(action.label());
        if action == ForgeThreadAction::Reply {
            let r = ui.add_enabled(
                !busy,
                TextEdit::multiline(&mut t.body)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .char_limit(8192)
                    .hint_text("Reply saved locally until submitted"),
            );
            control(widgets, "hosting-thread-body", &r, ui.clip_rect());
        }
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                widgets,
                "hosting-thread-preview",
                "Review thread action",
                !busy && !t.unknown,
            ) {
                out.action = Some("pull-thread-preview");
            }
            if btn(
                ui,
                widgets,
                "hosting-thread-discard",
                "Discard reply draft",
                !busy && !t.unknown,
            ) {
                t.body.clear();
                t.action = None;
                t.selected.clear();
            }
        });
    }
}
