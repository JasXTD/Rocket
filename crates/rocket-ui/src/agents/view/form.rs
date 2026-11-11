use super::*;

pub(super) fn show(
    ctx: &Context,
    w: &mut Widgets,
    f: &mut Form,
    ready: bool,
    action: &mut Option<Action>,
) {
    let removing = f.mode == Mode::Remove;
    let title = if removing {
        "Remove worktree"
    } else if f.mode == Mode::Resume {
        "Resuming conversation"
    } else if f.mode == Mode::Restart {
        "Review and restart"
    } else {
        "New agent session"
    };
    let head = pages::ReviewHeader {
        image: Icon::Worktree,
        title: tr(title),
        subtitle: tr(if f.plan.is_some() {
            "Review session"
        } else {
            "Separate worktrees. Your local tools. Review changes before merging."
        }),
        danger: removing,
    };
    let response = pages::review(
        ctx,
        "agents-review-dialog",
        head,
        w,
        "agents-scroll",
        true,
        |ui, part, w| {
            if part == pages::Part::Footer {
                if f.mode==Mode::Resume {if f.task.is_some(){ui.spinner();ui.label(tr("Checking worktree and executables"));}return;}
                if f.plan.is_some() {
                    if button(
                        ui,
                        w,
                        "agents-confirm",
                        if removing {
                            "Remove clean worktree"
                        } else if f.mode == Mode::Restart {
                            "Start session"
                        } else {
                            "Create and start"
                        },
                        ready,
                    ) {
                        *action = Some(Action::Confirm);
                    }
                } else {
                    if button(
                        ui,
                        w,
                        "agents-review",
                        "Review session",
                        ready && f.task.is_none(),
                    ) {
                        *action = Some(Action::Preview);
                    }
                    if f.task.is_some() {
                        ui.spinner();
                        ui.label(tr("Checking worktree and executables"));
                    }
                }
                return;
            }
            if let Some(e) = &f.error {
                ui.colored_label(RED(), e);
            }
            if let Some(plan) = &f.plan {
                ui.strong(&plan.record.name);
                pages::field(ui, &tr("New branch"), &plan.record.branch, true);
                pages::field(ui, &tr("Base"), plan.target.as_str(), true);
                pages::field(
                    ui,
                    &tr("Worktree folder"),
                    &plan.record.destination.to_string_lossy(),
                    true,
                );
                if removing {
                    ui.label(tr("Only a clean, unlocked worktree can be removed. The branch and recovery reference are kept."));
                } else {
                    ui.separator();
                    ui.strong(tr("Agent executable"));
                    command(ui, &plan.record.command);
                    if let Some(p) = &plan.program {
                        ui.add(
                            Label::new(
                                RichText::new(format!("SHA-256: {}", p.digest))
                                    .monospace()
                                    .small()
                                    .color(HINT()),
                            )
                            .wrap(),
                        );
                    }
                    if let Some(setup) = &plan.record.setup {
                        ui.strong(tr("Setup command"));
                        command(ui, setup);
                    }
                    ui.label(tr("Starts external programs with your account's access. Existing files and credentials are not copied. Setup runs only when explicitly selected."));
                }
                return;
            }
            ui.add_enabled_ui(f.task.is_none(),|ui| {
            if !removing {
                let r=ui.checkbox(&mut f.record.tracking,tr("Track conversation activity"));pages::control(w,"agents-tracking",&r,ui.clip_rect());
                pages::note(ui,&tr("Provider events enable attention and exact conversation resume. Existing logins and configuration files stay unchanged. Codex may ask you to review the hooks in /hooks."));
                field(ui,w,"agents-name","Task name",&mut f.record.name,false);
                ui.label(tr("Provider"));
                ui.horizontal_wrapped(|ui|{for p in Provider::ALL{
                    let r=ui.selectable_label(f.record.provider==p,tr(p.label()));
                    pages::control(w,format!("agents-provider-{}",p.key()),&r,ui.clip_rect());
                    if r.clicked(){f.record.provider=p;f.record.command=p.command();}
                }});
            }
            if f.mode==Mode::Create {
                field(ui,w,"agents-base","Base",&mut f.record.base,false);
                field(ui,w,"agents-branch","New branch",&mut f.record.branch,false);
                field(ui,w,"agents-destination","Worktree folder",&mut f.destination,false);
            }else{ui.add(Label::new(&f.destination).wrap());ui.label(&f.record.branch);}
            if !removing {
                field(ui,w,"agents-program","Agent executable",&mut f.record.command.program,false);
                field(ui,w,"agents-arguments","Arguments · one per line, no shell expansion",&mut f.record.command.arguments,true);
                let r=ui.checkbox(&mut f.setup_enabled,tr("Run a setup command first"));pages::control(w,"agents-setup-enabled",&r,ui.clip_rect());
                if f.setup_enabled {
                    field(ui,w,"agents-setup-program","Setup executable",&mut f.setup.program,false);
                    field(ui,w,"agents-setup-arguments","Setup arguments",&mut f.setup.arguments,true);
                    pages::note(ui,&tr("The agent starts only after setup exits successfully. Setup is optional on every restart."));
                }
                pages::note(ui,&tr("Use your installed tool and existing login. Rocket does not install agents or bypass their permission prompts."));
            }
        });
        },
    );
    if let Some(close) = w.get("dialog-close").cloned() {
        w.insert("agents-cancel".into(), close);
    }
    // Resizing the dialog can move its boundary under the pointer. Only X/Escape
    // dismiss, never a click that became outside after a compact-window reflow.
    if response.close_clicked() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        *action = Some(Action::Cancel);
    }
}
