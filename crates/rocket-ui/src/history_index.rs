//! Speed up history: the notice above the History list when ordering it is slow, and
//! the dialog that writes, updates or removes Git's index of the repository's commits
//! (its commit-graph). All Git work stays on cancellable workers. The explanations are
//! in the guide under "Faster history".
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    i18n::{tr, trf, trn},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// History ordering slower than this, in a repository without an index, shows the notice.
pub(crate) const SLOW: Duration = Duration::from_secs(1);
/// Slow loads in a row before the notice shows: one busy moment is not enough.
pub(crate) const SLOW_LOADS: u8 = 2;
/// What the outcome calls the write, so the dialog can show the reason without it.
pub(crate) const WRITE_PHASE: &str = "Write the commit index";
/// Bytes of index for each commit, measured on a 100,500-commit repository (60.0).
const BYTES_A_COMMIT: u64 = 60;
/// A cautious rate for the estimate: that repository wrote 16,000 to 53,000 a second.
const COMMITS_A_SECOND: u64 = 15_000;

#[derive(Default)]
pub struct DialogState {
    task: Option<ReadTask<HistoryIndex>>,
    count: Option<ReadTask<u64>>,
    pub(crate) index: Option<HistoryIndex>,
    /// Every commit a reference reaches; `None` until counted, or when counting failed.
    pub(crate) commits: Option<u64>,
    /// The review's "keep it up to date" choice, until an index exists.
    keep: bool,
    pub(crate) applying: Option<HistoryIndexChange>,
    stopping: bool,
    started: Option<Instant>,
    /// How long ordering took when the write began, and the tab's refresh count then:
    /// a later refresh has the time with the index.
    before: Option<(Duration, u64)>,
    /// This dialog wrote the index, in this long.
    pub(crate) written: Option<Duration>,
    pub(crate) failure: Option<Failure>,
    notice: Option<String>,
}

pub(crate) struct Failure {
    change: HistoryIndexChange,
    reason: String,
    details: String,
}

/// "3.1 s" below ten seconds, whole seconds from there.
fn seconds(time: Duration) -> String {
    if time < Duration::from_secs(10) {
        format!("{:.1} s", time.as_secs_f64())
    } else {
        format!("{} s", time.as_secs())
    }
}

fn commits_text(commits: u64) -> String {
    // `{n}` is always the bare count; this phrase carries the grouped number.
    trn!(commits, "{found} commit", "{found} commits", found = pages::number_text(commits as usize))
}

/// About how long writing takes here, in steps coarse enough to hold on a busy machine.
fn estimate(commits: u64) -> String {
    let seconds = commits / COMMITS_A_SECOND;
    if seconds > 90 {
        trf!("About {n} min here", n = seconds.div_ceil(60))
    } else {
        trf!("About {n} s here", n = (seconds.div_ceil(5) * 5).max(5))
    }
}

/// The reason Git gave, without the step's name and the pointer to the console.
fn reason(outcome: &Outcome) -> String {
    let git = outcome
        .raw_output
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("fatal: ").or_else(|| line.strip_prefix("error: ")));
    let detail = outcome.details.first().map(|detail| {
        detail
            .split_once(": ")
            .filter(|(phase, _)| !phase.contains('.'))
            .map_or(detail.as_str(), |(_, rest)| rest)
    });
    // "unsupported:" is how a refusal is spelled for the console, not for this card.
    git.or(detail).unwrap_or(&outcome.summary).trim_start_matches("unsupported: ").to_owned()
}

fn unused_text(unused: HistoryIndexUnused) -> &'static str {
    match unused {
        HistoryIndexUnused::Shallow => tr("This is a shallow clone, and Git keeps no index for one."),
        HistoryIndexUnused::Replaced => {
            tr("This repository has replaced or grafted commits, and Git reads no index while it does.")
        }
        HistoryIndexUnused::TurnedOff => {
            tr("Git's core.commitGraph setting is off here, so Git would ignore the index.")
        }
    }
}

/// Whether this tab may show the notice: it can write the index, and the notice was
/// not closed for its repository.
pub(crate) fn offered(tab: &Tab, hidden: &std::collections::BTreeSet<std::path::PathBuf>) -> bool {
    !tab.is_preview() && !tab.review_window && !hidden.contains(&tab.path)
}

/// Slow loads in a row once this load is counted: one more than before it, or none
/// when this load was not slow. The notice waits for `SLOW_LOADS` of them.
pub(crate) fn slow_loads(before: Option<&crate::backend::Snapshot>, slow: bool) -> u8 {
    if slow { before.map_or(0, |snapshot| snapshot.slow_loads).saturating_add(1) } else { 0 }
}

/// The line above the History list: how long ordering took, the dialog, and a × that
/// hides it for this repository.
pub(crate) fn notice(
    ui: &mut Ui,
    tab: &Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(snapshot) = &tab.snapshot else {
        return;
    };
    let (true, Some(took)) = (snapshot.history_notice && snapshot.slow_loads >= SLOW_LOADS, snapshot.ordering) else {
        return;
    };
    let strip = Frame::new()
        .fill(BLUE().gamma_multiply(0.07))
        .inner_margin(Margin { left: 14, right: 10, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hide = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0)
                        .on_hover_text(tr("Hide this for this repository"));
                    pages::control(widgets, "history-speed-hide", &hide, ui.clip_rect());
                    if hide.clicked() {
                        intents.push(Intent::HideHistoryNotice);
                    }
                    let open = pages::primary(ui, true, tr("Speed up history"));
                    pages::control(widgets, "history-speed-open", &open, ui.clip_rect());
                    if open.clicked() {
                        intents.push(Intent::Dialog(DialogKind::HistoryIndex));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), mark, Icon::Bolt, BLUE());
                        ui.label(
                            RichText::new(trf!("Ordering this history took {time}.", time = seconds(took)))
                                .font(semibold(13.0))
                                .color(TEXT()),
                        );
                        ui.add(
                            Label::new(
                                RichText::new(tr("Git's index of its commits makes that faster."))
                                    .color(MUTED()),
                            )
                            .truncate(),
                        );
                    });
                });
            });
        })
        .response
        .rect;
    pages::control(widgets, "history-speed-notice", &ui.interact(strip, Id::new(("history-speed-notice", tab.id)), Sense::hover()), ui.clip_rect());
    ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, BLUE().gamma_multiply(0.28)));
}

/// One green tick and what it says is so.
fn tick(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (mark, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
        icon(ui.painter(), mark, Icon::Check, GREEN());
        ui.add(Label::new(RichText::new(text).color(TEXT())).wrap());
    });
}

/// "Keep it up to date when Rocket fetches", as a row of its own. Returns whether it
/// was switched.
fn keep_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    enabled: bool,
    keep: &mut bool,
) -> bool {
    pages::well(ui, |ui| {
        let r = ui.add_enabled(
            enabled,
            checkbox(keep, tr("Keep it up to date when Rocket fetches")),
        );
        pages::control(widgets, "dialog-history-index-keep", &r, ui.clip_rect());
        r.changed()
    })
}

impl Rocket {
    pub(crate) fn start_history_index_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::HistoryIndex) {
            return;
        }
        let s = d.history_index.get_or_insert_with(|| DialogState { keep: true, ..Default::default() });
        if s.applying.is_some() {
            return;
        }
        d.error = None;
        let path = self.tabs.iter().find(|t| Some(t.id) == d.owner).map(|t| t.path.clone());
        let (Some(backend), Some(path)) = (self.backend.clone(), path) else {
            d.error = Some(tr("Open a repository first.").into());
            return;
        };
        let (counter, counted) = (backend.clone(), path.clone());
        match crate::read_task::spawn(&self.context, move |cancel| backend.history_index(&path, &cancel)) {
            Ok(task) => s.task = Some(task),
            Err(error) => d.error = Some(user_error(&error)),
        }
        // Only the estimates and the progress total wait for the count.
        s.count = crate::read_task::spawn(&self.context, move |cancel| counter.count_commits(&counted, &cancel)).ok();
    }

    pub(crate) fn poll_history_index_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.history_index else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(index) => {
                    if index.file.is_some() {
                        s.keep = index.on_fetch;
                    }
                    s.index = Some(index);
                }
                Err(error) => d.error = Some(user_error(&error)),
            }
        }
        if let Some(result) = s.count.as_mut().and_then(ReadTask::try_finish) {
            s.count = None;
            s.commits = result.ok();
        }
    }

    pub(crate) fn finish_history_index_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.history_index else { return };
        if d.owner != owner {
            return;
        }
        let Some(change) = s.applying.take() else { return };
        let stopped = std::mem::take(&mut s.stopping);
        let took = s.started.take().map(|started| started.elapsed());
        s.failure = None;
        s.notice = None;
        if outcome.command_succeeded {
            match change {
                HistoryIndexChange::Write { .. } => s.written = took,
                HistoryIndexChange::Remove => {
                    s.written = None;
                    s.before = None;
                }
                HistoryIndexChange::OnFetch(_) => {}
            }
        } else {
            if matches!(change, HistoryIndexChange::Write { .. }) {
                s.before = None;
            }
            if stopped {
                s.notice = Some(tr("Stopped. The repository is as it was.").into());
            } else {
                s.failure = Some(Failure {
                    change,
                    reason: reason(outcome),
                    details: format!(
                        "{}\n{}\n{}",
                        outcome.summary,
                        outcome.details.join("\n"),
                        outcome.raw_output
                    )
                    .trim()
                    .to_owned(),
                });
            }
        }
        self.start_history_index_read();
    }

    pub(crate) fn history_index_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.history_index.get_or_insert_with(|| DialogState { keep: true, ..Default::default() });
        if s.applying.is_none() && d.owner != self.tab().map(|t| t.id) {
            return;
        }
        let tab = self.tabs.iter().find(|t| Some(t.id) == d.owner);
        let name = tab.map(|t| t.title.clone()).unwrap_or_default();
        let ordering = tab.and_then(|t| t.snapshot.as_ref()).and_then(|snapshot| snapshot.ordering);
        let refreshes = tab.map_or(0, |t| t.refresh_count);
        let writable = tab.is_some_and(|t| !t.is_preview() && !t.review_window);
        let idle = self.write.is_none() && !self.quitting && writable;
        let running = pages::Running::of(self.write.as_ref());
        let (progress, fraction) = self
            .write
            .as_ref()
            .map_or((String::new(), None), |w| (w.progress.clone(), w.fraction));
        let writing = matches!(s.applying, Some(HistoryIndexChange::Write { .. }));
        let file = s.index.as_ref().and_then(|index| index.file.clone());
        let unused = s.index.as_ref().and_then(|index| index.unused);
        // The time with the index: the first refresh after the write began.
        let after = s
            .before
            .filter(|(_, at)| refreshes > *at && s.written.is_some())
            .and(ordering);
        let (mut dismiss, mut stop, mut copy) = (false, false, false);
        let mut change = None;
        let response = pages::review(
            ctx,
            "rocket-history-index",
            pages::ReviewHeader {
                image: Icon::Bolt,
                title: tr("Speed up history"),
                subtitle: tr("Writes Git's index of this repository's commits."),
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-history-index-scroll",
            s.applying.is_none(),
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    let mut facts = vec![name.clone()];
                    if let Some(commits) = s.commits.or(file.as_ref().and_then(|file| file.commits)) {
                        facts.push(commits_text(commits));
                    }
                    // In Done the two times have a card of their own.
                    if let Some(took) = ordering.filter(|_| !writing && s.written.is_none()) {
                        facts.push(trf!("ordering took {time}", time = seconds(took)));
                    }
                    let r = ui.label(RichText::new(facts.join(" · ")).size(12.0).color(MUTED()));
                    pages::control(widgets, "dialog-history-index-facts", &r, ui.clip_rect());
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, BLUE(), notice);
                    }
                    if writing {
                        pages::well(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(tr("Writing the index")).font(semibold(13.0)));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let r = ui.label(RichText::new(&progress).size(12.0).color(MUTED()));
                                    pages::control(widgets, "dialog-history-index-progress", &r, ui.clip_rect());
                                });
                            });
                            // Git says how far it is only once it has counted for a second;
                            // until then the footer's spinner is the sign of life.
                            if let Some(fraction) = fraction {
                                ui.add(ProgressBar::new(fraction).desired_height(6.0));
                            }
                        });
                        ui.label(
                            RichText::new(tr("Stopping leaves the repository as it was."))
                                .size(12.0)
                                .color(MUTED()),
                        );
                    } else if let Some(failure) = &s.failure {
                        let r = Frame::new()
                            .fill(AMBER().gamma_multiply(0.06))
                            .stroke(Stroke::new(1.0, AMBER().gamma_multiply(0.45)))
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                    icon(ui.painter(), mark, Icon::Warning, AMBER());
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.label(
                                            RichText::new(match failure.change {
                                                // Also when Rocket refused before Git ran, so it does not blame Git.
                                                HistoryIndexChange::Write { .. } => tr("The index was not written"),
                                                HistoryIndexChange::Remove => tr("The index could not be removed"),
                                                HistoryIndexChange::OnFetch(_) => tr("The setting could not be saved"),
                                            })
                                            .font(semibold(13.0)),
                                        );
                                        ui.add(Label::new(RichText::new(&failure.reason).color(TEXT_SOFT())).wrap());
                                    });
                                });
                            })
                            .response;
                        pages::control(widgets, "dialog-history-index-failure", &r, ui.clip_rect());
                        tick(ui, tr("Commits, branches and your files stay as they are"));
                    } else if s.index.is_none() {
                        if d.error.is_none() {
                            pages::waiting(ui, tr("Reading the index"));
                        }
                    } else if let Some(file) = &file {
                        if let Some(unused) = unused {
                            pages::callout(ui, AMBER(), unused_text(unused));
                        }
                        let size = crate::explore::size(file.bytes);
                        if let Some((before, _)) = s.before.filter(|_| s.written.is_some()) {
                            pages::well(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.label(RichText::new(tr("Ordering history")).font(semibold(13.0)));
                                        ui.label(
                                            RichText::new(tr("Measured on this repository"))
                                                .size(12.0)
                                                .color(MUTED()),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing.x = 10.0;
                                        // Green and a struck-out old time only for a real gain: in a
                                        // small repository both times are mostly Git starting up.
                                        let faster = after.is_some_and(|after| after < before);
                                        let r = match after {
                                            Some(after) => ui.label(
                                                RichText::new(seconds(after))
                                                    .font(semibold(16.0))
                                                    .color(if faster { GREEN() } else { TEXT() }),
                                            ),
                                            None => ui.add(Spinner::new().size(14.0)),
                                        };
                                        pages::control(widgets, "dialog-history-index-after", &r, ui.clip_rect());
                                        let (arrow, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                        icon(ui.painter(), arrow, Icon::ChevronRight, MUTED());
                                        let before = RichText::new(seconds(before)).color(MUTED());
                                        ui.label(if faster { before.strikethrough() } else { before });
                                    });
                                });
                            });
                        }
                        if s.written.is_some() {
                            tick(ui, &trf!("Index written, {size}", size));
                            if s.index.as_ref().is_some_and(|index| index.on_fetch) {
                                tick(ui, tr("Kept up to date when Rocket fetches"));
                            }
                        } else {
                            pages::well(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        let when = file
                                            .written
                                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                                            .map(|since| {
                                                crate::activity_history::short_date(
                                                    since.as_secs() as i64,
                                                    crate::activity_history::now_ms(),
                                                )
                                            });
                                        ui.label(
                                            RichText::new(match when {
                                                Some(date) => trf!("Index written {date}", date),
                                                None => tr("Index written").into(),
                                            })
                                            .font(semibold(13.0)),
                                        );
                                        let mut about = vec![size.clone()];
                                        about.extend(file.commits.map(commits_text));
                                        ui.label(RichText::new(about.join(" · ")).size(12.0).color(MUTED()));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let r = if unused.is_some() {
                                            pages::pill(ui, tr("Not in use"), AMBER())
                                        } else {
                                            pages::pill(ui, tr("In use"), GREEN())
                                        };
                                        pages::control(widgets, "dialog-history-index-state", &r, ui.clip_rect());
                                    });
                                });
                            });
                            let mut keep = s.keep;
                            if keep_row(ui, widgets, idle && s.applying.is_none(), &mut keep) {
                                change = Some(HistoryIndexChange::OnFetch(keep));
                            }
                        }
                    } else {
                        if let Some(unused) = unused {
                            pages::callout(ui, AMBER(), unused_text(unused));
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            tick(
                                ui,
                                &match s.commits {
                                    Some(commits) => trf!(
                                        "Adds one file inside .git, about {size}",
                                        size = crate::explore::size(commits.saturating_mul(BYTES_A_COMMIT))
                                    ),
                                    None => tr("Adds one file inside .git").into(),
                                },
                            );
                            tick(ui, tr("Commits, branches and your files stay as they are"));
                            tick(ui, tr("You can remove it here at any time"));
                        });
                        keep_row(ui, widgets, s.applying.is_none(), &mut s.keep);
                    }
                }
                pages::Part::Footer if s.applying.is_some() => {
                    stop = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let ready = idle && s.task.is_none() && unused.is_none();
                    if let Some(failure) = &s.failure {
                        let r = pages::primary(ui, idle, tr("Try again"));
                        pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                        if r.clicked() {
                            change = Some(failure.change);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let r = pages::ghost(ui, true, tr("Copy details"));
                            pages::control(widgets, "dialog-history-index-copy", &r, ui.clip_rect());
                            copy = r.clicked();
                        });
                    } else if s.index.is_none() {
                        // Still reading: nothing to decide yet.
                    } else if file.is_none() {
                        let r = pages::primary(ui, ready, tr("Write index"));
                        pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                        if r.clicked() {
                            change = Some(HistoryIndexChange::Write { on_fetch: s.keep, commits: s.commits });
                        }
                        // Below that many commits the write is over before an estimate helps.
                        if let Some(commits) = s.commits.filter(|commits| *commits >= COMMITS_A_SECOND) {
                            pages::footer_status(ui, false, &estimate(commits));
                        }
                    } else if let Some(took) = s.written {
                        let r = pages::ghost(ui, idle, tr("Remove index"));
                        pages::control(widgets, "dialog-history-index-remove", &r, ui.clip_rect());
                        if r.clicked() {
                            change = Some(HistoryIndexChange::Remove);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let r = pages::pill(ui, tr("Written"), GREEN());
                            pages::control(widgets, "dialog-history-index-written", &r, ui.clip_rect());
                            ui.label(RichText::new(seconds(took)).size(12.5).color(HINT()));
                        });
                    } else {
                        let r = pages::primary(ui, ready, tr("Update index"));
                        pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                        if r.clicked() {
                            change = Some(HistoryIndexChange::Write { on_fetch: s.keep, commits: s.commits });
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let r = pages::danger_ghost(ui, idle, tr("Remove index"));
                            pages::control(widgets, "dialog-history-index-remove", &r, ui.clip_rect());
                            if r.clicked() {
                                change = Some(HistoryIndexChange::Remove);
                            }
                        });
                    }
                }
            },
        );
        // Reads and results resize this dialog, so a click outside is not a dismissal;
        // the header's × and Escape are.
        dismiss |= (response.close_clicked()
            || (ctx.input(|i| i.key_pressed(Key::Escape)) && response.should_close()))
            && s.applying.is_none();
        if stop {
            s.stopping = true;
            self.cancel_write();
        }
        if copy && let Some(failure) = &s.failure {
            ctx.copy_text(failure.details.clone());
        }
        if let (Some(change), Some(owner)) = (change, d.owner) {
            if matches!(change, HistoryIndexChange::Write { .. }) {
                s.before = ordering.map(|took| (took, refreshes));
                s.written = None;
            }
            s.started = Some(Instant::now());
            s.applying = Some(change);
            s.notice = None;
            self.execute(owner, Action::HistoryIndex(change));
            if self.write.is_none() {
                s.applying = None;
                d.error = Some(tr("Wait for repository work to finish, then review again.").into());
            } else {
                s.failure = None;
                d.error = None;
            }
        }
        if !dismiss {
            self.dialog = Some(d);
        }
    }
}

#[cfg(test)]
mod tests;
