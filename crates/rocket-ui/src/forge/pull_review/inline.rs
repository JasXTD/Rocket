//! Local review drafts retain the location and revision selected in the diff.
use super::*;

pub struct Selection {
    pub file: usize,
    pub path: String,
    pub head: String,
    pub base: String,
    pub diff_revision: Option<String>,
    pub line: u64,
    pub start: u64,
    pub side: ForgeDiffSide,
}

pub(super) fn queued(
    ui: &mut Ui,
    s: &mut super::super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
) {
    ui.colored_label(MUTED(), "Inline comments are saved on this computer until you submit or discard the review. Select an available diff line to add a comment.");
    if s.pull.details.as_ref().is_some_and(|d|d.repository().config().provider()==ForgeProvider::GitLab) {
        ui.colored_label(MUTED(), "GitLab: send one single-line discussion with Comment and an empty review message. Submit other discussions or votes separately.");
    } else if s.pull.details.as_ref().is_some_and(|d|d.repository().config().provider()==ForgeProvider::Gitea) {
        ui.colored_label(MUTED(), "Gitea: up to 32 single-line comments in one reviewed submission.");
    } else if s.pull.details.as_ref().is_some_and(|d|matches!(d.repository().config().provider(),ForgeProvider::Bitbucket|ForgeProvider::BitbucketServer|ForgeProvider::Azure|ForgeProvider::AzureServer)) {
        ui.colored_label(MUTED(), "Send one single-line comment at a time. Select Comment and leave the separate review message empty. Refresh to see published comments.");
        if s.pull.details.as_ref().is_some_and(|d|d.repository().config().provider()==ForgeProvider::Bitbucket) {
            ui.colored_label(MUTED(), "Bitbucket Cloud cannot pin a comment to a commit. Rocket checks for changes before and after sending; inspect any uncertain result before retrying.");
        }
    }
    let mut edit = None;
    let mut remove = None;
    for (i, c) in s.pull.comments.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!(
                "{} · {} lines {}–{}",
                c.path(),
                c.side().key(),
                c.start_line(),
                c.line()
            ));
            if btn(
                ui,
                widgets,
                &format!("hosting-inline-edit-{i}"),
                "Edit",
                !busy,
            ) {
                edit = Some(i);
            }
            if btn(
                ui,
                widgets,
                &format!("hosting-inline-remove-{i}"),
                "Remove",
                !busy,
            ) {
                remove = Some(i);
            }
        });
        ui.add(Label::new(c.body()).wrap());
        if s.pull
            .details
            .as_ref()
            .is_some_and(|d| d.summary().head != c.head())
        {
            ui.colored_label(AMBER(), "This comment belongs to an earlier head. Remove it and review the new diff before submitting.");
        }
    }
    if let Some(i) = remove {
        s.pull.comments.remove(i);
        if let Some(editing) = s.pull.inline_editing {
            if editing == i {
                s.pull.inline_editing = None;
                s.pull.inline_body.clear();
            } else if editing > i {
                s.pull.inline_editing = Some(editing - 1);
            }
        }
    } else if let Some(i) = edit {
        if !s.pull.inline_body.is_empty() {
            s.error = Some("Save or cancel the current line draft first.".into());
        } else {
            s.pull.inline_body = s.pull.comments[i].body().into();
            s.pull.inline_editing = Some(i);
            s.pull.inline_selection = None;
            s.pull.tab = 2;
        }
    }
}

pub(super) fn files(
    ui: &mut Ui,
    details: &ForgePullDetails,
    s: &mut super::super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
) {
    collection_note(ui, &details.files);
    let edit = !busy
        && s.pull.preview.is_none()
        && s.pull.merge.preview.is_none()
        && s.pull.unknown.is_none()
        && s.pull.merge.unknown.is_none()
        && details.summary().state == "open"
        && !details.summary().merged
        && details.supports_inline_review();
    for (i, f) in details.files.items.iter().enumerate() {
        let r = ui.add(
            Button::selectable(
                s.pull.file == i,
                format!(
                    "{}  {}  +{} −{}",
                    f.status, f.path, f.additions, f.deletions
                ),
            )
            .wrap(),
        );
        control(
            widgets,
            format!("hosting-pull-file-{i}"),
            &r,
            ui.clip_rect(),
        );
        if r.clicked() {
            s.pull.file = i;
        }
    }
    // Keep the editor above the diff so a long file does not hide a draft.
    if s.pull.inline_selection.is_some() || s.pull.inline_editing.is_some() {
        ui.separator();
        if let Some(selection) = &mut s.pull.inline_selection {
            ui.strong(format!(
                "Draft · {} · {} lines {}–{}",
                selection.path,
                selection.side.key(),
                selection.start,
                selection.line
            ));
            if selection.head != details.summary().head || selection.base != details.summary().base
            {
                ui.colored_label(
                    AMBER(),
                    "The PR changed. Cancel this draft and select a line from the refreshed diff.",
                );
            }
            if details.supports_inline_ranges() { ui.horizontal_wrapped(|ui| {
                ui.label("First line");let r=ui.add_enabled(edit,DragValue::new(&mut selection.start).range(1..=u32::MAX as u64));control(widgets,"hosting-inline-start",&r,ui.clip_rect());
                ui.label("Last line");let r=ui.add_enabled(edit,DragValue::new(&mut selection.line).range(1..=u32::MAX as u64));control(widgets,"hosting-inline-end",&r,ui.clip_rect());
            }); } else { ui.label(format!("{} line {}",selection.side.key(),selection.line)); }
        } else {
            ui.strong("Edit queued comment (keeps its original revision)");
        }
        let r = ui.add_enabled(
            edit,
            TextEdit::multiline(&mut s.pull.inline_body)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .char_limit(8192)
                .hint_text("Comment on the selected line"),
        );
        control(widgets, "hosting-inline-body", &r, ui.clip_rect());
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                widgets,
                "hosting-inline-save",
                "Save to review draft",
                edit && !s.pull.inline_body.trim().is_empty(),
            ) {
                let result = if let Some(i) = s.pull.inline_editing {
                    s.pull
                        .comments
                        .get_mut(i)
                        .ok_or_else(|| {
                            Error::InvalidInput(
                                "The comment was removed. Select a line again.".into(),
                            )
                        })
                        .and_then(|c| c.edit_body(&s.pull.inline_body))
                } else if let Some(selection) = &s.pull.inline_selection {
                    if selection.head != details.summary().head
                        || selection.base != details.summary().base
                        || selection.diff_revision != details.inline_diff_revision()
                        || details
                            .files
                            .items
                            .get(selection.file)
                            .is_none_or(|f| f.path != selection.path)
                    {
                        Err(Error::InvalidInput(
                            "The diff changed. Cancel this draft and select a line again.".into(),
                        ))
                    } else if s.pull.comments.len() >= details.inline_comment_limit() {
                        Err(Error::InvalidInput(
                            "This provider's comment limit is reached. Review and submit the current draft before adding another.".into(),
                        ))
                    } else if s.pull.comments.iter().any(|c| {
                        c.path() == selection.path
                            && c.line() == selection.line
                            && c.side() == selection.side
                    }) {
                        Err(Error::InvalidInput(
                            "This line already has a draft comment. Edit that comment instead."
                                .into(),
                        ))
                    } else {
                        details
                            .inline_comment_range(
                                selection.file,
                                selection.start,
                                selection.line,
                                selection.side,
                                &s.pull.inline_body,
                            )
                            .map(|c| s.pull.comments.push(c))
                    }
                } else {
                    Err(Error::InvalidInput("Select a diff line first.".into()))
                };
                match result {
                    Ok(()) => {
                        s.pull.inline_body.clear();
                        s.pull.inline_selection = None;
                        s.pull.inline_editing = None;
                        s.pull.composing = true;
                        s.error = None;
                    }
                    Err(e) => s.error = Some(user_error(&e)),
                }
            }
            if btn(
                ui,
                widgets,
                "hosting-inline-cancel",
                "Cancel line draft",
                !busy,
            ) {
                s.pull.inline_body.clear();
                s.pull.inline_selection = None;
                s.pull.inline_editing = None;
            }
        });
    }
    let Some(file) = details.files.items.get(s.pull.file) else {
        return;
    };
    ui.separator();
    ui.strong(&file.path);
    if let Some(previous) = &file.previous {
        ui.label(format!("Renamed from {previous}"));
    }
    if details.supports_inline_review() {
        ui.colored_label(MUTED(), "Left numbers refer to the old file; right numbers refer to the new file. Click a number to comment. This preview never changes local files.");
    } else {
        ui.colored_label(AMBER(), "The provider did not supply the revision data needed for inline comments. Refresh the PR or open the complete diff on the provider.");
    }
    if file.lines.is_empty() {
        ui.colored_label(AMBER(), "Inline comments are unavailable for a binary, missing, oversized or incomplete patch. Open the provider for the complete diff.");
        if let Some(patch) = &file.patch {
            ui.add(Label::new(RichText::new(patch).monospace()).wrap());
        }
    } else {
        for (row, line) in file.lines.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                for (side, number, key) in [
                    (ForgeDiffSide::Left, line.old, "left"),
                    (ForgeDiffSide::Right, line.new, "right"),
                ] {
                    if let Some(number) = number
                        && btn(
                            ui,
                            widgets,
                            &format!("hosting-inline-line-{}-{row}-{key}", s.pull.file),
                            &format!("{key} {number}"),
                            edit,
                        )
                    {
                        // Never silently replace text the user is already writing.
                        if !s.pull.inline_body.is_empty() {
                            s.error = Some("Save or cancel the current line draft first.".into());
                        } else {
                            s.pull.inline_selection = Some(Selection {
                                file: s.pull.file,
                                path: file.path.clone(),
                                head: details.summary().head.clone(),
                                base: details.summary().base.clone(),
                                diff_revision: details.inline_diff_revision(),
                                line: number,
                                start: number,
                                side,
                            });
                            s.pull.inline_editing = None;
                            s.pull.composing = true;
                            s.error = None;
                        }
                    }
                }
                let color = match line.text.as_bytes().first() {
                    Some(b'+') => GREEN(),
                    Some(b'-') => RED(),
                    Some(b'@') => BLUE(),
                    _ => TEXT(),
                };
                ui.add(Label::new(RichText::new(&line.text).monospace().color(color)).wrap());
            });
        }
    }
}
