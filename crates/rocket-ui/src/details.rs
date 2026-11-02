//! Selected commit: message, metadata, changed files and diff, visible together.
//! Narrow panes fall back to a Details / Files switch.
use crate::{
    app::{DetailTab, DialogKind, Intent, Tab, UiControl},
    patch_view::patch_view,
    style::*,
};
use eframe::egui::*;
use rocket_git::{Commit, CommitActionRequest, CommitOperation};
use std::collections::HashMap;

pub(crate) fn commit_details(
    ui: &mut Ui,
    tab: &mut Tab,
    widgets: &mut HashMap<String, UiControl>,
    intents: &mut Vec<Intent>,
    commands: &[rocket_git::CustomCommand],
) {
    let commit = tab
        .selected_commit
        .as_ref()
        .and_then(|id| {
            tab.commits()
                .iter()
                .chain(tab.loaded_commits())
                .find(|c| &c.id == id)
        })
        .cloned();
    let Some(commit) = commit else {
        Frame::new().inner_margin(16).show(ui, |ui| {
            ui.label(RichText::new("Select a commit to inspect it.").color(FAINT()));
        });
        return;
    };
    Panel::top(Id::new(("commit-review-actions",tab.id))).frame(Frame::new().fill(VIEW_PANEL()).inner_margin(6)).show(ui,|ui| {
        ui.horizontal_wrapped(|ui| {
            let review=ui.button("Open review window");widgets.insert("commit-open-review".into(),(&review).into());
            if review.clicked(){intents.push(Intent::OpenReview(crate::reviews::Request{commit:commit.id.clone(),path:None,blame:false}));}
            let link=ui.button("Commit link");widgets.insert("commit-browser-link".into(),(&link).into());
            if link.clicked(){intents.push(Intent::CommitLink(commit.id.clone()));}
        });
    });
    if ui.available_width() >= 720.0 {
        let meta = Panel::left(Id::new(("commit-meta", tab.id)))
            .default_size(360.0)
            .size_range(260.0..=(ui.available_width() * 0.5).max(260.0))
            .resizable(true)
            .frame(Frame::new().fill(VIEW_PANEL()))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt(("commit-metadata", tab.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let header =
                            commit_header(ui, &commit, &mut tab.issue_links, widgets, intents);
                        widgets.insert("detail-details".into(), (&header).into());
                        if header.clicked() {
                            tab.detail_tab = DetailTab::Details;
                        }
                        merge_fold(ui, tab, &commit, widgets, intents);
                        file_list(ui, tab, widgets, intents, commands);
                    });
            })
            .response
            .rect;
        ui.painter().vline(
            meta.right() - 0.5,
            meta.y_range(),
            Stroke::new(1.0, HAIRLINE()),
        );
        CentralPanel::default()
            .frame(Frame::new().fill(VIEW_BG()))
            .show(ui, |ui| {
                if let Some(intent) = patch_view(ui, tab, false, false, widgets) {
                    intents.push(intent);
                }
            });
        return;
    }
    let strip = Frame::new()
        .fill(VIEW_PANEL())
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                for (kind, label, key) in [
                    (DetailTab::Details, "Details".to_string(), "detail-details"),
                    (
                        DetailTab::Files,
                        format!(
                            "Files ({})",
                            tab.patch.as_ref().map_or(0, |p| p.files.len())
                        ),
                        "detail-files",
                    ),
                ] {
                    let selected = tab.detail_tab == kind
                        || (kind == DetailTab::Details && tab.detail_tab == DetailTab::Checks);
                    let response = ui.selectable_label(selected, label);
                    widgets.insert(key.into(), (&response).into());
                    if response.clicked() {
                        tab.detail_tab = kind;
                    }
                }
            });
        })
        .response
        .rect;
    ui.painter().hline(
        strip.x_range(),
        strip.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    if tab.detail_tab == DetailTab::Files {
        if let Some(intent) = patch_view(ui, tab, true, false, widgets) {
            intents.push(intent);
        }
    } else {
        ScrollArea::vertical()
            .id_salt(("commit-metadata", tab.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                commit_header(ui, &commit, &mut tab.issue_links, widgets, intents);
                merge_fold(ui, tab, &commit, widgets, intents);
                if file_list(ui, tab, widgets, intents, commands) {
                    tab.detail_tab = DetailTab::Files;
                }
            });
    }
}

/// For a merge in the history list: collapse it there (hide the commits it brought
/// in) or expand it again, as ← and → do in the list.
fn merge_fold(
    ui: &mut Ui,
    tab: &Tab,
    commit: &Commit,
    widgets: &mut HashMap<String, UiControl>,
    intents: &mut Vec<Intent>,
) {
    if commit.parents.len() < 2 || !tab.search.trim().is_empty() {
        return;
    }
    let collapsed = tab.folding.collapsed(commit);
    Frame::new()
        .inner_margin(Margin::symmetric(14, 6))
        .show(ui, |ui| {
            let r = ui
                .button(if collapsed {
                    "Expand this merge in the list"
                } else {
                    "Collapse this merge in the list"
                })
                .on_hover_text(
                    "Hide or show the commits this merge brought in (← and → in the commit list)",
                );
            widgets.insert("detail-merge-fold".into(), (&r).into());
            if r.clicked() {
                intents.push(Intent::FoldMerge(commit.id.clone(), !collapsed));
            }
        });
}

/// Subject, body, author, IDs and signature check. Returns the clickable subject.
/// Issue IDs that the repository's rules find become links.
pub(crate) fn commit_header(
    ui: &mut Ui,
    commit: &Commit,
    links: &mut crate::issue_links::State,
    widgets: &mut HashMap<String, UiControl>,
    intents: &mut Vec<Intent>,
) -> Response {
    let summary = subject(commit);
    let message = display(&commit.message);
    let body = message.split_once('\n').map_or("", |(_, body)| body.trim());
    let (summary_links, body_links) = links.message(&commit.id, &summary, body);
    let (summary_links, body_links) = (summary_links.to_vec(), body_links.to_vec());
    let output = Frame::new()
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let title = ui
                .horizontal_top(|ui| {
                    // The subject wraps at the left; the actions button stays top-right.
                    let text_width = (ui.available_width() - 34.0).max(60.0);
                    let title = ui
                        .allocate_ui(vec2(text_width, 0.0), |ui| {
                            ui.set_width(text_width);
                            if summary_links.is_empty() {
                                return ui.add(
                                    Label::new(RichText::new(&summary).font(semibold(15.0)))
                                        .wrap()
                                        .sense(Sense::click()),
                                );
                            }
                            let (title, url) = crate::issue_links::label(
                                ui,
                                &summary,
                                &summary_links,
                                semibold(15.0),
                                TEXT(),
                                Sense::click(),
                                widgets,
                                0,
                            );
                            intents.extend(url.map(Intent::OpenIssueLink));
                            title
                        })
                        .inner;
                    {
                        let more = crate::chrome::icon_button(ui, Icon::More, true, false, 26.0)
                            .on_hover_text("Commit actions");
                        Popup::menu(&more).show(|ui| {
                            if ui.button("Copy commit ID").clicked() {
                                ui.ctx().copy_text(commit.id.to_string());
                                ui.close();
                            }
                            if ui.button("Browse files at this commit").clicked() {
                                intents.push(Intent::Tree(crate::tree::TreeIntent::OpenSelected));
                                ui.close();
                            }
                            ui.separator();
                            for (operation, label) in [
                                (CommitOperation::CherryPick, "Cherry-pick"),
                                (CommitOperation::Revert, "Revert"),
                            ] {
                                if ui.button(label).clicked() {
                                    intents.push(Intent::Dialog(DialogKind::CommitAction(
                                        CommitActionRequest {
                                            operation,
                                            targets: vec![commit.id.clone()],
                                            mainline: None,
                                            recovery: None,
                                        },
                                    )));
                                    ui.close();
                                }
                            }
                        });
                    }
                    title
                })
                .inner;
            if !body.is_empty() && body_links.is_empty() {
                ui.add(Label::new(RichText::new(body).size(12.5).color(MUTED())).wrap());
            } else if !body.is_empty() {
                let (_, url) = crate::issue_links::label(
                    ui,
                    body,
                    &body_links,
                    FontId::proportional(12.5),
                    MUTED(),
                    Sense::hover(),
                    widgets,
                    summary_links.len(),
                );
                intents.extend(url.map(Intent::OpenIssueLink));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let name = display(&commit.author.name);
                avatar(ui, &name);
                ui.label(RichText::new(&name).size(12.5));
                ui.label(
                    RichText::new(display(&commit.author.email))
                        .size(12.0)
                        .color(FAINT()),
                );
            });
            ui.label(
                RichText::new(date(commit.author.timestamp, commit.author.offset_minutes))
                    .size(12.0)
                    .color(FAINT()),
            );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if id_chip(ui, &commit.id.as_str()[..8], true)
                    .on_hover_text("Copy full commit ID")
                    .clicked()
                {
                    ui.ctx().copy_text(commit.id.to_string());
                }
                for parent in &commit.parents {
                    ui.label(RichText::new("parent").size(12.0).color(FAINT()));
                    if id_chip(ui, &parent.as_str()[..8], false)
                        .on_hover_text("Copy full parent ID")
                        .clicked()
                    {
                        ui.ctx().copy_text(parent.to_string());
                    }
                }
                // Inside the header's margins, next to the IDs it checks.
                let r = ui.button("Verify signature");
                widgets.insert("detail-verify-signature".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::Dialog(DialogKind::Signing(Some(
                        rocket_git::SignatureTarget::Commit(commit.id.clone()),
                    ))));
                }
            });
            if commit.parents.len() > 1 {
                ui.label(
                    RichText::new("Merge diff against the first parent")
                        .size(12.0)
                        .color(FAINT()),
                );
            }
            title
        });
    let rect = output.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    output.inner
}

/// Changed files of the selected commit. Returns true when a file was chosen.
fn file_list(ui: &mut Ui, tab: &mut Tab, widgets: &mut HashMap<String, UiControl>, intents:&mut Vec<Intent>, commands:&[rocket_git::CustomCommand]) -> bool {
    let mut chosen = false;
    let context=crate::custom_commands::context(tab);
    let (count, added, removed) = tab.patch.as_ref().map_or((0, 0, 0), |p| {
        (
            p.files.len(),
            p.files.iter().map(|f| f.added).sum::<usize>(),
            p.files.iter().map(|f| f.removed).sum::<usize>(),
        )
    });
    let (rect, header) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    header.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Files changed"));
    widgets.insert("detail-files".into(), (&header).into());
    if header.clicked() {
        tab.detail_tab = DetailTab::Files;
    }
    let galley = ui
        .painter()
        .layout_no_wrap("Files changed".into(), semibold(12.5), MUTED());
    let width = galley.size().x;
    ui.painter().galley(
        pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        MUTED(),
    );
    ui.painter().text(
        pos2(rect.left() + 20.0 + width, rect.center().y),
        Align2::LEFT_CENTER,
        count.to_string(),
        FontId::proportional(12.0),
        FAINT(),
    );
    let removed_text = format!("−{removed}");
    let removed_width = ui
        .painter()
        .layout_no_wrap(removed_text.clone(), FontId::monospace(11.5), RED())
        .size()
        .x;
    ui.painter().text(
        pos2(rect.right() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        removed_text,
        FontId::monospace(11.5),
        RED(),
    );
    ui.painter().text(
        pos2(rect.right() - 20.0 - removed_width, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("+{added}"),
        FontId::monospace(11.5),
        GREEN(),
    );
    let Some(patch) = &tab.patch else {
        // Text only: the diff pane beside it shows the one spinner for this load.
        if tab.diff.is_some() {
            Frame::new()
                .inner_margin(Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.label(RichText::new("Loading changes…").color(FAINT()));
                });
        }
        return false;
    };
    Frame::new()
        .inner_margin(Margin {
            left: 6,
            right: 6,
            top: 0,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, file) in patch.files.iter().enumerate() {
                let response = ui
                    .add(
                        Button::new("")
                            .frame(false)
                            .min_size(vec2(ui.available_width(), 30.0)),
                    )
                    .on_hover_text(&file.label);
                let r = response.rect;
                let selected = i == tab.patch_file;
                row_background(ui.painter(), r, selected, response.hovered());
                let (dir, name) = file.label.rsplit_once('/').unwrap_or(("", &file.label));
                let numbers = format!("+{}  −{}", file.added, file.removed);
                let numbers_width = ui
                    .painter()
                    .layout_no_wrap(numbers.clone(), FontId::monospace(11.5), FAINT())
                    .size()
                    .x;
                let right = r.right() - numbers_width - 16.0;
                let name_galley =
                    ui.painter()
                        .layout_no_wrap(name.into(), FontId::proportional(13.0), TEXT());
                let name_width = name_galley.size().x.min(right - r.left() - 10.0).max(0.0);
                ui.painter()
                    .with_clip_rect(Rect::from_min_max(r.min, pos2(right, r.bottom())))
                    .galley(
                        pos2(r.left() + 10.0, r.center().y - name_galley.size().y / 2.0),
                        name_galley,
                        TEXT(),
                    );
                if !dir.is_empty() && r.left() + 18.0 + name_width < right - 16.0 {
                    clipped_text(
                        ui,
                        Rect::from_min_max(
                            pos2(r.left() + 18.0 + name_width, r.top()),
                            pos2(right, r.bottom()),
                        ),
                        dir,
                        12.0,
                        FAINT(),
                    );
                }
                ui.painter().text(
                    pos2(r.right() - 10.0 - numbers_width, r.center().y),
                    Align2::LEFT_CENTER,
                    format!("+{}", file.added),
                    FontId::monospace(11.5),
                    GREEN(),
                );
                let plus_width = ui
                    .painter()
                    .layout_no_wrap(format!("+{}  ", file.added), FontId::monospace(11.5), GREEN())
                    .size()
                    .x;
                ui.painter().text(
                    pos2(r.right() - 10.0 - numbers_width + plus_width, r.center().y),
                    Align2::LEFT_CENTER,
                    format!("−{}", file.removed),
                    FontId::monospace(11.5),
                    RED(),
                );
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &file.label));
                if response.interact_rect.is_positive() {
                    widgets.insert(
                        format!("patch-file-{i}"),
                        UiControl {
                            rect: response.interact_rect,
                            enabled: response.enabled(),
                        },
                    );
                }
                if response.clicked() {
                    tab.patch_file = i;
                    chosen = true;
                }
                if let (Some(commit),Some(path))=(&patch.command_commit,&file.command_path)
                    && tab.selected_commit.as_ref()==Some(commit) && file.submodule.is_none() {
                    let context=crate::custom_commands::historical_context(&context,commit,path.as_bytes());
                    response.context_menu(|ui| crate::custom_commands::context_menu(ui,commands,rocket_git::CommandScope::File,&context,intents,widgets));
                }
            }
        });
    chosen
}

fn avatar(ui: &mut Ui, name: &str) {
    let initials: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|s| s.chars().next())
        .collect();
    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 10.0, AVATAR());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials.to_uppercase(),
        semibold(9.5),
        TEXT(),
    );
}

fn id_chip(ui: &mut Ui, text: &str, filled: bool) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), FontId::monospace(12.0), TEXT());
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(galley.size() + vec2(16.0, 8.0)),
    );
    let rect = response.rect;
    if filled {
        ui.painter()
            .rect_filled(rect, 6.0, if response.hovered() { HOVER() } else { RAISED() });
    } else {
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, if response.hovered() { MUTED() } else { BORDER() }),
            StrokeKind::Inside,
        );
    }
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if filled { TEXT() } else { MUTED() },
    );
    response
}
