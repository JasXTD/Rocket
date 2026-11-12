//! A selected PR's bounded, read-only Hosting snapshot. Replacing/dropping the
//! task cancels it; a late response has no receiver in the next selection.
use super::*;
use crate::{
    backend::Backend,
    i18n::{mark, tr, trf, trn},
};

#[derive(Default)]
pub struct State {
    pub number: Option<u64>,
    pub data: Option<Arc<ForgePullDetails>>,
    pub error: Option<String>,
    pub stopping: bool,
    pub tab: usize,
    task: Option<ReadTask<ForgePullDetails>>,
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            self.stopping = true;
            task.cancel();
        }
    }
    pub(super) fn start(
        &mut self,
        number: u64,
        connection: pulls::Connection,
        backend: Backend,
        ctx: &Context,
    ) {
        // Even a retry clears the previous revision; never show stale details as fresh.
        *self = Self {
            number: Some(number),
            ..Default::default()
        };
        let (repository, session) = connection;
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.forge_request("insights-detail", &session, |s| {
                let details = s.pull_details(&repository, number, &cancel)?;
                cancel.check()?;
                Ok(details)
            })
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub(super) fn poll(&mut self) {
        let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        self.task = None;
        let result = if self.stopping {
            Err(Error::Cancelled)
        } else {
            result
        };
        self.stopping = false;
        match result {
            Ok(data) if Some(data.summary().number) == self.number => {
                self.data = Some(Arc::new(data))
            }
            Ok(_) => self.error = Some(tr("PR selection changed. Open the row again.").into()),
            Err(Error::Cancelled) => {
                self.error = Some(tr("Stopped reading details. The PR list is unchanged.").into())
            }
            Err(error) => self.error = Some(user_error(&error)),
        }
    }
}

/// What a collection lacks: a read that failed, a bounded preview, or nothing in it.
fn coverage<T>(
    ui: &mut Ui,
    c: &ForgePullCollection<T>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if let Some(error) = &c.error {
        pulls::strip(ui, AMBER(), &user_error(&Error::Forge(*error)), None, widgets);
    }
    if c.limited && c.error.is_none() {
        pulls::strip(
            ui,
            AMBER(),
            tr("This collection is incomplete. The host has more items or could not return all data."),
            None,
            widgets,
        );
    }
    if c.items.is_empty() && c.error.is_none() && !c.limited {
        ui.label(RichText::new(tr("No items returned.")).size(12.5).color(HINT()));
    }
}
fn text(ui: &mut Ui, value: &str) {
    // API text is plain data. No Markdown links, image requests or executable content.
    let end = value
        .char_indices()
        .nth(8192)
        .map_or(value.len(), |(n, _)| n);
    ui.add(
        Label::new(RichText::new(&value[..end]).color(TEXT_SOFT()))
            .wrap()
            .selectable(true),
    );
    if end < value.len() {
        ui.label(
            RichText::new(tr("Text preview limited to 8,192 characters."))
                .size(12.0)
                .color(HINT()),
        );
    }
}
/// A host's ISO date as "2026-11-10 15:30 UTC"; anything else as the host wrote it.
fn when(raw: &str) -> String {
    let b = raw.as_bytes();
    let shaped = b.len() >= 16
        && b[..16].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T' || *c == b' ',
            13 => *c == b':',
            _ => c.is_ascii_digit(),
        });
    if !shaped {
        return raw.to_owned();
    }
    // Seconds and fractions are left out; what follows them is the zone.
    let zone = raw[16..].trim_start_matches(|c: char| c == ':' || c == '.' || c.is_ascii_digit());
    let zone = match zone {
        "Z" | "z" => " UTC".to_owned(),
        "" => String::new(),
        other => format!(" {other}"),
    };
    format!("{} {}{zone}", &raw[..10], &raw[11..16])
}
/// A section's small heading with how many it holds.
fn heading(ui: &mut Ui, title: &str, count: usize) {
    let mut job = text::LayoutJob::default();
    job.append(
        &title.to_uppercase(),
        0.0,
        TextFormat {
            extra_letter_spacing: 0.6,
            ..TextFormat::simple(semibold(11.0), HINT())
        },
    );
    job.append(
        &format!("  {count}"),
        0.0,
        TextFormat::simple(FontId::proportional(11.0), HINT()),
    );
    ui.add_space(6.0);
    ui.label(job);
}
/// A branch, commit or path on a chip fill, cut short where there is no room.
fn chip(ui: &mut Ui, value: &str) -> Response {
    ui.add(
        Label::new(
            RichText::new(format!(" {value} "))
                .monospace()
                .size(11.5)
                .color(TEXT_SOFT())
                .background_color(CHIP()),
        )
        .truncate()
        .selectable(true),
    )
}
/// A one-line label that takes at most `width` of its row and is cut short beyond
/// it, so a long name leaves room for what follows.
fn capped(ui: &mut Ui, width: f32, text: RichText) -> Response {
    let layout = *ui.layout();
    ui.allocate_ui_with_layout(vec2(width.max(24.0), 18.0), layout, |ui| {
        ui.add(Label::new(text).truncate())
    })
    .inner
}
/// The hairline under a row of a tab.
fn rule(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(line.x_range(), line.center().y, Stroke::new(1.0, CARD_LINE()));
}
/// The pull request's state as shown and its colour.
fn state(p: &ForgePullSummary) -> (String, Color32) {
    if p.merged {
        return (tr("Merged").to_owned(), VIOLET());
    }
    let open = matches!(
        p.state.trim().to_lowercase().as_str(),
        "open" | "opened" | "active" | "new"
    );
    if p.draft && open {
        (tr("Draft").to_owned(), MUTED())
    } else {
        crate::forge::state_look(&p.state)
    }
}
/// A review's verdict as shown and its colour; a word Rocket doesn't know stays as
/// the host wrote it.
fn verdict(state: &str) -> (String, Color32) {
    match state.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
        "approved" | "approve" => (tr("Approved").to_owned(), GREEN()),
        "changes_requested" | "request_changes" | "needs_work" | "rejected" => {
            (tr("Changes requested").to_owned(), RED())
        }
        "commented" | "comment" => (tr("Commented").to_owned(), MUTED()),
        "dismissed" => (tr("Dismissed").to_owned(), MUTED()),
        "pending" => (tr("Pending").to_owned(), AMBER()),
        _ => (state.to_owned(), MUTED()),
    }
}
/// A changed file's kind as the letter Changes uses, and its colour.
fn kind(status: &str) -> (&'static str, Color32) {
    match status.trim().to_lowercase().as_str() {
        "added" | "add" | "new" => ("A", GREEN()),
        "removed" | "deleted" | "delete" => ("D", RED()),
        "renamed" | "rename" | "moved" => ("R", VIOLET()),
        "copied" | "copy" => ("C", VIOLET()),
        _ => ("M", AMBER()),
    }
}

/// A message centred in `height`: nothing chosen yet.
fn hint(ui: &mut Ui, height: f32, words: &str) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::top_down(Align::Center),
        |ui| {
            ui.set_min_size(vec2(width, height));
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.add_space(((height - 96.0) / 2.0).max(12.0));
            let (mark, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().circle_filled(mark.center(), 20.0, RAISED());
            icon(ui.painter(), mark.shrink(11.0), Icon::Merge, HINT());
            ui.allocate_ui_with_layout(
                vec2(300.0_f32.min(width - 32.0), 18.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add(
                        Label::new(RichText::new(words).size(12.5).color(HINT()))
                            .wrap()
                            .halign(Align::Center),
                    );
                },
            );
        },
    );
}

/// The selected pull request as a card. With `fixed` it is that tall and what its tab
/// shows scrolls inside it; without, it is as tall as its contents and the page
/// scrolls. Returns whether to read the details again.
pub(super) fn show(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    fixed: Option<f32>,
) -> bool {
    let mut retry = false;
    let mut close = false;
    let top = ui.cursor().top();
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(height) = fixed {
                ui.set_min_height(height - 2.0);
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            // What is left under the cursor of a fixed card.
            let rest = |ui: &Ui| fixed.map(|h| (top + h - 1.0 - ui.cursor().top()).max(0.0));
            let Some(number) = s.number else {
                hint(
                    ui,
                    rest(ui).unwrap_or(160.0),
                    tr("Select a pull request to read its description, files, discussion and checks."),
                );
                return;
            };
            let data = s.data.clone();
            let busy = s.running();
            pages::card_bar(ui, |ui| {
                ui.label(RichText::new(format!("#{number}")).font(semibold(13.0)));
                if let Some(data) = &data {
                    let (word, color) = state(data.summary());
                    pages::pill(ui, &word, color);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                        .on_hover_text(tr("Close details"));
                    pages::control(widgets, "insights-detail-close", &r, ui.clip_rect());
                    close = r.clicked();
                    let r = crate::chrome::icon_button(ui, Icon::Refresh, !busy, false, 28.0)
                        .on_hover_text(tr("Read details again"));
                    pages::control(widgets, "insights-detail-refresh", &r, ui.clip_rect());
                    retry |= r.clicked();
                });
            });
            if busy {
                // One spinner, with Stop where the eye already is.
                let stopping = s.stopping;
                let mut stop = false;
                pages::loading_with(
                    ui,
                    tr(if stopping {
                        mark("Stopping…")
                    } else {
                        mark("Reading details…")
                    }),
                    38.0,
                    |ui| {
                        ui.add_space(8.0);
                        let r = ui.add_enabled(!stopping, Button::new(tr("Stop")));
                        pages::control(widgets, "insights-detail-stop", &r, ui.clip_rect());
                        stop = r.clicked();
                    },
                );
                if stop {
                    s.cancel();
                }
                return;
            }
            if let Some(error) = &s.error {
                Frame::new()
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        retry |= pulls::strip(
                            ui,
                            AMBER(),
                            error,
                            Some(("insights-detail-retry", tr("Read again"), true)),
                            widgets,
                        );
                    });
            }
            let Some(data) = data else { return };
            let p = data.summary();
            Frame::new()
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 7.0;
                    // Beside the list the card has a fixed height, so a long title
                    // takes two lines at most and leaves the tab its room; pointing
                    // at it shows all of it.
                    let mut title = text::LayoutJob::simple(
                        p.title.clone(),
                        semibold(15.0),
                        TEXT(),
                        f32::INFINITY,
                    );
                    if fixed.is_some() {
                        title.wrap.max_rows = 2;
                    }
                    let r = ui.add(Label::new(title).wrap().selectable(true));
                    if fixed.is_some() {
                        r.on_hover_text(&p.title);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(&p.author).size(12.5));
                        chip(ui, &p.source);
                        ui.label(RichText::new("→").size(12.5).color(HINT()));
                        chip(ui, &p.target);
                    });
                    if p.counts_available {
                        let plain = TextFormat::simple(FontId::proportional(12.5), HINT());
                        let mut counts = text::LayoutJob::default();
                        counts.append(
                            &format!(
                                "{} · {} · ",
                                trn!(p.commits, "{n} commit", "{n} commits"),
                                trn!(p.files, "{n} file", "{n} files")
                            ),
                            0.0,
                            plain.clone(),
                        );
                        counts.append(
                            &format!("+{}", p.additions),
                            0.0,
                            TextFormat::simple(semibold(12.5), GREEN()),
                        );
                        counts.append(" ", 0.0, plain);
                        counts.append(
                            &format!("−{}", p.deletions),
                            0.0,
                            TextFormat::simple(semibold(12.5), RED()),
                        );
                        ui.label(counts);
                    } else {
                        ui.label(
                            RichText::new(tr("Change counts are unavailable from this host."))
                                .size(12.5)
                                .color(HINT()),
                        );
                    }
                    ui.add_space(2.0);
                    // A count only where the read knows it.
                    let counts = [
                        None,
                        Some(if p.counts_available {
                            p.files as usize
                        } else {
                            data.files.items.len()
                        }),
                        Some(data.discussion.items.len() + data.inline_comments.items.len()),
                        (data.checks.error.is_none() || data.statuses.error.is_none())
                            .then(|| data.checks.items.len() + data.statuses.items.len()),
                    ];
                    let names = [mark("Summary"), mark("Files"), mark("Discussion"), mark("Checks")];
                    // The counts go when, with them, the four tabs would no longer
                    // fit on one row: a narrow card, large counts or longer words.
                    let needed: f32 = names
                        .into_iter()
                        .zip(counts)
                        .map(|(name, count)| {
                            let label = match count {
                                Some(n) => format!("{}  {}", tr(name), pages::number_text(n)),
                                None => tr(name).to_owned(),
                            };
                            let galley = ui.painter().layout_no_wrap(
                                label,
                                FontId::proportional(12.5),
                                TEXT(),
                            );
                            galley.size().x + 20.0
                        })
                        .sum();
                    let roomy = needed + 10.0 <= ui.available_width();
                    let tabs: Vec<(&str, Option<usize>, bool)> = names
                        .into_iter()
                        .zip(counts)
                        .enumerate()
                        .map(|(i, (name, count))| (tr(name), count.filter(|_| roomy), s.tab == i))
                        .collect();
                    for (i, r) in pages::tab_segments(ui, &tabs).iter().enumerate() {
                        pages::control(
                            widgets,
                            format!("insights-detail-tab-{i}"),
                            r,
                            ui.clip_rect(),
                        );
                        if r.clicked() {
                            s.tab = i;
                        }
                    }
                });
            rule(ui);
            let tab = s.tab;
            let mut body = |ui: &mut Ui| {
                Frame::new()
                    .inner_margin(Margin {
                        left: 16,
                        right: 16,
                        top: 8,
                        bottom: 16,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        match tab {
                            1 => files(ui, &data, widgets),
                            2 => discussion(ui, &data, widgets),
                            3 => checks(ui, &data, widgets),
                            _ => summary(ui, &data, widgets),
                        }
                    });
            };
            match rest(ui) {
                Some(height) => {
                    let scroll = ScrollArea::vertical()
                        // Each pull request and tab starts at its top.
                        .id_salt(("insights-detail-scroll", number, tab))
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, body);
                    pages::region(
                        widgets,
                        "insights-detail-scroll",
                        scroll.inner_rect.intersect(ui.clip_rect()),
                        true,
                    );
                }
                None => body(ui),
            }
        });
    if close {
        *s = Default::default();
        retry = false;
    }
    retry
}

/// The description, then the commits and the reviews.
fn summary(ui: &mut Ui, data: &ForgePullDetails, widgets: &mut HashMap<String, UiControl>) {
    let p = data.summary();
    ui.add_space(4.0);
    pages::well(ui, |ui| {
        if p.body.is_empty() {
            ui.label(RichText::new(tr("No description.")).color(HINT()));
        } else {
            text(ui, &p.body);
        }
    });
    heading(ui, tr("Commits"), data.commits.items.len());
    coverage(ui, &data.commits, widgets);
    for c in &data.commits.items {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 9.0;
            chip(ui, &c.id[..c.id.len().min(8)]);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let room = (ui.available_width() * 0.4).min(160.0);
                capped(ui, room, RichText::new(&c.author).size(12.0).color(FAINT()));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(&c.subject).truncate().selectable(true));
                });
            });
        });
    }
    heading(ui, tr("Reviews"), data.reviews.items.len());
    coverage(ui, &data.reviews, widgets);
    for r in &data.reviews.items {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 9.0;
            let room = ui.available_width() * 0.45;
            capped(ui, room, RichText::new(&r.author).font(semibold(13.0)));
            let (word, color) = verdict(&r.state);
            pages::pill(ui, &word, color);
            ui.add(Label::new(RichText::new(when(&r.date)).size(12.0).color(FAINT())).truncate());
        });
        if !r.body.is_empty() {
            text(ui, &r.body);
        }
    }
}

/// Each changed file: its kind, its path (and the one it was renamed from), then
/// the lines added and removed.
fn files(ui: &mut Ui, data: &ForgePullDetails, widgets: &mut HashMap<String, UiControl>) {
    coverage(ui, &data.files, widgets);
    ui.spacing_mut().item_spacing.y = 0.0;
    for f in &data.files.items {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 9.0;
            ui.set_min_height(if f.previous.is_some() { 46.0 } else { 32.0 });
            let (letter, color) = kind(&f.status);
            let (badge, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            ui.painter().rect_filled(badge, 4.0, color.gamma_multiply(0.14));
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                letter,
                semibold(11.0),
                color,
            );
            response.on_hover_text(&f.status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("−{}", f.deletions))
                        .monospace()
                        .size(12.0)
                        .color(RED()),
                );
                ui.label(
                    RichText::new(format!("+{}", f.additions))
                        .monospace()
                        .size(12.0)
                        .color(GREEN()),
                );
                // Centred in the row like the mark and the counts: a block placed
                // top-down would start at the row's top instead.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let path = Label::new(RichText::new(&f.path).monospace().size(12.0))
                        .truncate()
                        .selectable(true);
                    let Some(old) = &f.previous else {
                        ui.add(path);
                        return;
                    };
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 34.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(path);
                            ui.add(
                                Label::new(
                                    RichText::new(trf!("from {path}", path = old))
                                        .size(12.0)
                                        .color(FAINT()),
                                )
                                .truncate()
                                .selectable(true),
                            );
                        },
                    );
                });
            });
        });
        rule(ui);
    }
}

/// One comment: who wrote it and when, the line it is on, then its text.
fn comment(ui: &mut Ui, c: &ForgePullComment) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        let room = ui.available_width() * 0.45;
        capped(ui, room, RichText::new(&c.author).font(semibold(13.0)));
        ui.add(Label::new(RichText::new(when(&c.date)).size(12.0).color(FAINT())).truncate());
    });
    if let Some(path) = &c.path {
        match c.line {
            Some(line) => chip(ui, &trf!("{path} · line {line}", path, line)),
            None => chip(ui, path),
        };
    }
    text(ui, &c.body);
    ui.add_space(4.0);
    rule(ui);
}

/// The conversation, then the comments on lines.
fn discussion(ui: &mut Ui, data: &ForgePullDetails, widgets: &mut HashMap<String, UiControl>) {
    heading(ui, tr("Conversation"), data.discussion.items.len());
    coverage(ui, &data.discussion, widgets);
    for c in &data.discussion.items {
        comment(ui, c);
    }
    heading(ui, tr("Line comments"), data.inline_comments.items.len());
    coverage(ui, &data.inline_comments, widgets);
    for c in &data.inline_comments.items {
        comment(ui, c);
    }
}

/// Reported checks and statuses with the marks Hosting and Actions use. A missing
/// check is never shown as passed.
fn checks(ui: &mut Ui, data: &ForgePullDetails, widgets: &mut HashMap<String, UiControl>) {
    for (title, collection) in [(mark("Checks"), &data.checks), (mark("Statuses"), &data.statuses)] {
        heading(ui, tr(title), collection.items.len());
        coverage(ui, collection, widgets);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for c in &collection.items {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    ui.set_min_height(32.0);
                    let (spot, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    crate::ci_checks::paint_check(
                        ui.painter(),
                        spot.center(),
                        crate::forge::pull_review::pull_check_state(c),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(c.conclusion.as_deref().unwrap_or(&c.status))
                                .size(12.0)
                                .color(FAINT()),
                        );
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(&c.name).truncate().selectable(true));
                        });
                    });
                });
                rule(ui);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn states_verdicts_and_file_kinds_read_as_words_and_letters() {
        assert_eq!(verdict("APPROVED").0, "Approved");
        assert_eq!(verdict("changes requested").0, "Changes requested");
        assert_eq!(verdict("CHANGES_REQUESTED").1, RED());
        // A word Rocket doesn't know is shown as the host wrote it.
        assert_eq!(verdict("Waiting for author").0, "Waiting for author");
        assert_eq!(kind("renamed").0, "R");
        assert_eq!(kind("removed").0, "D");
        assert_eq!(kind("added").0, "A");
        assert_eq!(kind("changed").0, "M");
        // Dates read as a day and a time; a shape Rocket doesn't know stays as it is.
        assert_eq!(when("2026-11-10T15:30:53.753Z"), "2026-11-10 15:30 UTC");
        assert_eq!(when("2026-11-10T15:30:53+08:00"), "2026-11-10 15:30 +08:00");
        assert_eq!(when("2026-11-10 15:30"), "2026-11-10 15:30");
        for raw in ["", "yesterday", "1731252653000", "2026-11-10T15:3", "資料資料資料資料資料資料"] {
            assert_eq!(when(raw), raw);
        }
    }
}
