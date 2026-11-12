//! Pull request trends: opened and merged per week from the loaded sample, and the chosen
//! period beside the one before it. Weeks the sample is known not to reach are drawn as
//! not loaded, and a partial sample that can't say which weeks it reaches says it may miss
//! some; the periods are compared only when both are loaded.
use super::{
    pulls::{self, Comparison},
    *,
};
use crate::i18n::{tr, trf, trn};

/// The chart card's least height; in a wide page the cards take the rest of the window.
const CHART_MIN: f32 = 240.0;
/// The comparison card's width beside the chart.
const COMPARE: f32 = 440.0;

/// What a click in the view asked for.
#[derive(Default)]
pub(super) struct Asked {
    /// Load the next pages, so the sample reaches the previous period.
    pub more: bool,
    /// Read the list again: it changed while it was read.
    pub again: bool,
}

/// The chart and the comparison under the tiles: side by side and filling `rest` (the
/// page height left) when `wide`, else one under the other.
pub(super) fn show(
    ui: &mut Ui,
    s: &pulls::State,
    widgets: &mut HashMap<String, UiControl>,
    wide: bool,
    rest: f32,
) -> Asked {
    let mut asked = Asked::default();
    let compare = !matches!(s.comparison(), Comparison::None);
    if wide {
        let height = rest.max(CHART_MIN);
        let width = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let chart = if compare { width - COMPARE - 16.0 } else { width };
            ui.allocate_ui_with_layout(vec2(chart, height), Layout::top_down(Align::Min), |ui| {
                chart_card(ui, s, widgets, height);
            });
            if compare {
                ui.allocate_ui_with_layout(vec2(COMPARE, height), Layout::top_down(Align::Min), |ui| {
                    comparison_card(ui, s, widgets, &mut asked);
                });
            }
        });
    } else {
        chart_card(ui, s, widgets, CHART_MIN);
        if compare {
            comparison_card(ui, s, widgets, &mut asked);
        }
    }
    asked
}

/// The chart in a card `height` tall: its title, what a bar is, the legend, the bars.
fn chart_card(ui: &mut Ui, s: &pulls::State, widgets: &mut HashMap<String, UiControl>, height: f32) {
    let (title, note) = match s.trend.unit {
        ForgeTrendUnit::Week => (tr("Opened and merged per week"), tr("weeks from Monday, UTC")),
        ForgeTrendUnit::Month => (tr("Opened and merged per month"), tr("months, UTC")),
        ForgeTrendUnit::Year => (tr("Opened and merged per year"), tr("years, UTC")),
    };
    // More exists and nothing says which weeks the sample holds in full.
    let note = if s.more && s.coverage == ForgePullCoverage::Unknown {
        format!("{note} · {}", tr("some may be missing from this sample"))
    } else {
        note.to_owned()
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect(rect, 10.0, PANEL(), Stroke::new(1.0, HAIRLINE()), StrokeKind::Inside);
    let inner = Rect::from_min_max(rect.min + vec2(16.0, 14.0), rect.max - vec2(16.0, 14.0));
    let mut card = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    card.set_clip_rect(inner.expand(2.0).intersect(ui.clip_rect()));
    card.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // Right to left: Merged, then Opened, so they read in that order.
            for (label, color) in [(tr("Merged"), VIOLET()), (tr("Opened"), BLUE())] {
                ui.label(RichText::new(label).size(12.0).color(MUTED()));
                let (swatch, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(swatch, 2.0, color);
                ui.add_space(6.0);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(title).font(semibold(13.5)));
                ui.add(Label::new(RichText::new(note.as_str()).size(12.0).color(HINT())).truncate());
            });
        });
    });
    card.add_space(8.0);
    let plot_height = (inner.bottom() - card.cursor().top()).max(80.0);
    plot(&mut card, s, widgets, plot_height);
}

/// Whether a bucket starts before the sample holds every pull request, so its counts
/// would be too low.
fn not_loaded(s: &pulls::State, bucket: &ForgeTrendBucket) -> bool {
    matches!(s.coverage, ForgePullCoverage::After(_)) && !s.coverage.covers(bucket.start)
}

/// The day of a Unix time, with its year when that isn't this year's: "Sep 2" or
/// "Sep 2, 2025".
fn day_label(seconds: u64, now: u64) -> String {
    let day = (seconds / 86_400) as i64;
    if year_of(day) == year_of((now / 86_400) as i64) {
        month_day(day)
    } else {
        format!("{}, {}", month_day(day), year_of(day))
    }
}

/// The days from `start` to before `end`: "May 16 – Aug 13", with years when they differ.
fn range_label(start: u64, end: u64) -> String {
    let (first, last) = ((start / 86_400) as i64, (end.saturating_sub(1) / 86_400) as i64);
    if year_of(first) == year_of(last) {
        format!("{} – {}", month_day(first), month_day(last))
    } else {
        format!("{}, {} – {}, {}", month_day(first), year_of(first), month_day(last), year_of(last))
    }
}

/// A month bucket's name and year.
fn month_label(start: u64) -> (String, String) {
    let when = date(start as i64, 0);
    let month: usize = when[5..7].parse().unwrap_or(1);
    (crate::i18n::tr(MONTHS[month.clamp(1, 12) - 1]).to_owned(), when[..4].to_owned())
}

/// Under a bar: the first day of a week, a month (with the year in January and on the
/// first bar), or a year.
fn axis_label(unit: ForgeTrendUnit, bucket: &ForgeTrendBucket, first: bool) -> String {
    match unit {
        ForgeTrendUnit::Week => month_day((bucket.start / 86_400) as i64),
        ForgeTrendUnit::Month => {
            let (month, year) = month_label(bucket.start);
            if first || date(bucket.start as i64, 0)[5..7] == *"01" {
                format!("{month} {year}")
            } else {
                month
            }
        }
        ForgeTrendUnit::Year => year_of((bucket.start / 86_400) as i64),
    }
}

/// A bar's tooltip: which week, month or year, and its two counts.
fn tooltip(s: &pulls::State, bucket: &ForgeTrendBucket) -> String {
    if not_loaded(s, bucket) {
        let ForgePullCoverage::After(oldest) = s.coverage else { return String::new() };
        return trf!(
            "Not loaded: the sample reaches back to {date}.",
            date = day_label(oldest, s.read_at)
        );
    }
    let day = (bucket.start / 86_400) as i64;
    let when = match s.trend.unit {
        ForgeTrendUnit::Week => {
            let monday = day - (day + 3).rem_euclid(7);
            if monday == day {
                trf!("Week of {date}", date = month_day(monday))
            } else {
                trf!("Week of {date}, from {start}", date = month_day(monday), start = month_day(day))
            }
        }
        ForgeTrendUnit::Month => {
            let (month, year) = month_label(bucket.start);
            format!("{month} {year}")
        }
        ForgeTrendUnit::Year => year_of(day),
    };
    format!(
        "{when}\n{} · {}",
        trn!(bucket.opened, "{n} opened", "{n} opened"),
        trn!(bucket.merged, "{n} merged", "{n} merged")
    )
}

/// Diagonal lines across `region`: weeks the sample doesn't reach.
fn hatch(painter: &Painter, region: Rect) {
    let painter = painter.with_clip_rect(region.intersect(painter.clip_rect()));
    painter.rect_filled(region, 3.0, HOVER().gamma_multiply(0.6));
    let stroke = Stroke::new(1.0, HINT().gamma_multiply(0.22));
    let mut x = region.left() - region.height();
    while x < region.right() {
        painter.line_segment([pos2(x, region.bottom()), pos2(x + region.height(), region.top())], stroke);
        x += 9.0;
    }
}

/// The bars: opened and merged side by side for each bucket, on gridlines with their
/// values, every few buckets labelled, the one under the pointer described.
fn plot(ui: &mut Ui, s: &pulls::State, widgets: &mut HashMap<String, UiControl>, height: f32) {
    let buckets = &s.trend.buckets;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    pages::control(widgets, "insights-trend-chart", &response, ui.clip_rect());
    let (opened, merged) = buckets
        .iter()
        .fold((0, 0), |(o, m), b| (o + b.opened, m + b.merged));
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            true,
            format!(
                "{} · {}",
                trn!(opened, "{n} opened", "{n} opened"),
                trn!(merged, "{n} merged", "{n} merged")
            ),
        )
    });
    let painter = ui.painter_at(rect);
    if buckets.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("No pull request was opened or merged in this period."),
            FontId::proportional(13.0),
            MUTED(),
        );
        return;
    }
    // The weeks the sample doesn't reach come first, as the list reads newest first.
    let missing = buckets.iter().take_while(|b| not_loaded(s, b)).count();
    let max = buckets[missing..]
        .iter()
        .map(|b| b.opened.max(b.merged))
        .max()
        .unwrap_or(0)
        .max(1);
    let grid = grid_step(max);
    let top = max.div_ceil(grid) * grid;
    let plot = Rect::from_min_max(
        pos2(rect.left() + 30.0, rect.top() + 6.0),
        pos2(rect.right() - 4.0, rect.bottom() - 22.0),
    );
    let mut value = 0;
    while value <= top {
        let y = plot.bottom() - value as f32 / top as f32 * plot.height();
        painter.hline(plot.x_range(), y, Stroke::new(1.0, if value == 0 { BORDER() } else { HAIRLINE() }));
        painter.text(pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, value.to_string(), FontId::proportional(11.0), FAINT());
        value += grid;
    }
    let slot = plot.width() / buckets.len() as f32;
    let bar = ((slot * 0.8 - 2.0) / 2.0).clamp(1.5, 14.0);
    // Labels about 70 points apart, so they never overlap.
    let every = (70.0 / slot).ceil().max(1.0) as usize;
    if missing > 0 {
        let region = Rect::from_min_max(plot.min, pos2(plot.left() + slot * missing as f32, plot.bottom()));
        hatch(&painter, region);
        let galley = painter.layout_no_wrap(tr("Not loaded").into(), FontId::proportional(11.0), HINT());
        if galley.size().x + 24.0 <= region.width() {
            let pill = Rect::from_center_size(
                pos2(region.center().x, region.top() + 14.0),
                galley.size() + vec2(16.0, 4.0),
            );
            painter.rect(pill, 9.0, PANEL(), Stroke::new(1.0, HAIRLINE()), StrokeKind::Inside);
            painter.galley(pill.center() - galley.size() / 2.0, galley, HINT());
        }
    }
    let pointer = response.hover_pos();
    let mut hovered = None;
    for (i, b) in buckets.iter().enumerate() {
        let x = plot.left() + slot * (i as f32 + 0.5);
        let column = Rect::from_center_size(pos2(x, plot.center().y), vec2(slot, plot.height()));
        if pointer.is_some_and(|p| column.contains(p)) {
            hovered = Some(i);
            if i >= missing {
                painter.rect_filled(column, 2.0, HOVER());
            }
        }
        if i >= missing {
            for (left, count, color) in [(x - 1.0 - bar, b.opened, BLUE()), (x + 1.0, b.merged, VIOLET())] {
                if count > 0 {
                    let h = (count as f32 / top as f32 * plot.height()).max(2.0);
                    painter.rect_filled(
                        Rect::from_min_size(pos2(left, plot.bottom() - h), vec2(bar, h)),
                        CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 },
                        color,
                    );
                }
            }
        }
        if i % every == 0 {
            painter.text(
                pos2(x, rect.bottom() - 10.0),
                Align2::CENTER_CENTER,
                axis_label(s.trend.unit, b, i == 0),
                FontId::proportional(11.0),
                FAINT(),
            );
        }
    }
    if let Some(i) = hovered {
        response.on_hover_text_at_pointer(tooltip(s, &buckets[i]));
    }
}

/// The chosen period beside the one before it, or why they can't be compared yet and
/// the way to make them comparable.
fn comparison_card(
    ui: &mut Ui,
    s: &pulls::State,
    widgets: &mut HashMap<String, UiControl>,
    asked: &mut Asked,
) {
    let chosen = s.period.unwrap_or(Period::Days30);
    card(ui, chosen.compared_title(), "", |ui| {
        let Some((start, end)) = s.previous_range() else { return };
        let range = range_label(start, end);
        let (text, action) = match s.comparison() {
            Comparison::Ready(previous) => {
                table(ui, s, previous);
                return;
            }
            Comparison::None => return,
            Comparison::Changed => (
                tr("The list changed on the host while it was read, so the periods can't be compared.").to_owned(),
                Some(("insights-compare-again", tr("Read again"))),
            ),
            Comparison::NotLoaded(oldest) => {
                let more = pulls::more_left(s);
                let text = match (oldest, more) {
                    (Some(oldest), true) => trf!(
                        "The loaded sample reaches back to {date}, so {range} can't be compared yet.",
                        date = day_label(oldest, s.read_at),
                        range
                    ),
                    (Some(oldest), false) => trf!(
                        "The loaded sample reaches back to {date}, and this view stops at 500 pull requests, so {range} can't be compared.",
                        date = day_label(oldest, s.read_at),
                        range
                    ),
                    // The host lists by update, but a row has no update time to go by.
                    (None, true) if pulls::lists_by_update(s) => tr("Some loaded pull requests have no usable update time, so the periods can be compared only once every pull request is loaded.").to_owned(),
                    (None, false) if pulls::lists_by_update(s) => tr("Some loaded pull requests have no usable update time, and this view stops at 500 pull requests, so the periods can't be compared.").to_owned(),
                    (None, true) => tr("This host doesn't list pull requests by last update, so the periods can be compared only once every pull request is loaded.").to_owned(),
                    (None, false) => tr("This host doesn't list pull requests by last update, and this view stops at 500 pull requests, so the periods can't be compared.").to_owned(),
                };
                (text, more.then(|| ("insights-compare-more", tr("Load more"))))
            }
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some((id, label)) = action {
                    let r = ui.add_enabled(!s.running(), Button::new(label));
                    pages::control(widgets, id, &r, ui.clip_rect());
                    if r.clicked() {
                        if id == "insights-compare-more" {
                            asked.more = true;
                        } else {
                            asked.again = true;
                        }
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(text).size(12.5).color(MUTED())).wrap());
                });
            });
        });
    });
}

/// The value columns' widths: the two periods and the change.
const VALUE: f32 = 104.0;
const CHANGE: f32 = 60.0;

/// One table row: a label, then three right-aligned cells, each with an optional
/// second line under it; a hairline below.
fn table_row(ui: &mut Ui, label: RichText, cells: [(RichText, Option<String>); 3]) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let width = (ui.available_width() - VALUE * 2.0 - CHANGE).max(60.0);
        ui.allocate_ui_with_layout(vec2(width, 18.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_width(width);
            ui.add(Label::new(label).truncate());
        });
        for (i, (text, under)) in cells.into_iter().enumerate() {
            let w = if i == 2 { CHANGE } else { VALUE };
            ui.allocate_ui_with_layout(vec2(w, 18.0), Layout::top_down(Align::Max), |ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(text);
                if let Some(under) = under {
                    ui.add(Label::new(RichText::new(under).size(11.0).color(FAINT())).halign(Align::Max).wrap());
                }
            });
        }
    });
    ui.add_space(6.0);
    let y = ui.cursor().top() - 0.5;
    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, CARD_LINE()));
    ui.add_space(6.0);
}

/// Previous, the chosen period and the change for Opened, Merged and the merge time.
fn table(ui: &mut Ui, s: &pulls::State, previous: &ForgePullCounts) {
    let m = &s.metrics;
    let chosen = s.period.unwrap_or(Period::Days30);
    let Some((start, end)) = s.previous_range() else { return };
    ui.spacing_mut().item_spacing.y = 0.0;
    let head = |text: &str| RichText::new(text).size(12.0).color(HINT()).strong();
    table_row(
        ui,
        head(""),
        [
            (head(tr("Previous")), Some(range_label(start, end))),
            (head(chosen.last_label()), Some(range_label(end, s.read_at + 1))),
            (head(tr("Change")), None),
        ],
    );
    let plain = |text: String| RichText::new(text).size(12.5);
    let strong = |text: String| RichText::new(text).size(12.5).strong();
    let from = |n: usize| (n > 0).then(|| trn!(n, "from {n}", "from {n}"));
    for (label, before, now, moved, under) in [
        (tr("Opened"), previous.opened.to_string(), m.opened.to_string(), pulls::change(m.opened, previous.opened), (None, None)),
        (tr("Merged"), previous.merged.to_string(), m.merged.to_string(), pulls::change(m.merged, previous.merged), (None, None)),
        (
            tr("Median merge time"),
            pulls::duration(previous.median_merge_seconds),
            pulls::duration(m.median_merge_seconds),
            pulls::duration_change(m.median_merge_seconds, previous.median_merge_seconds).unwrap_or_else(|| "—".into()),
            (from(previous.merge_samples), from(m.merge_samples)),
        ),
    ] {
        table_row(
            ui,
            RichText::new(label).size(12.5),
            [(plain(before), under.0), (strong(now), under.1), (plain(moved), None)],
        );
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(ui.painter(), mark, Icon::Check, GREEN());
        ui.label(RichText::new(tr("Both periods are in the loaded sample.")).size(12.0).color(MUTED()));
    });
}
