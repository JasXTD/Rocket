//! The toolbar's branch switcher: a filter that has focus when the list opens, the
//! current branch first, then local branches newest first with their age and distance
//! from their upstream, and remote-tracking branches folded under one row until you
//! type. ↑ ↓ move the highlight, Enter chooses it and Esc closes the list.
use crate::{
    app::{DialogKind, Intent, Tab, UiControl, View},
    backend::Action,
    pages,
    style::*,
};
use eframe::egui::{text::LayoutJob, *};
use rocket_git::{Branch, BranchKind};
use std::collections::HashMap;

/// Most rows a group lists at once; the filter narrows longer lists.
const SHOWN: usize = 200;
/// A branch row's height.
const ROW: f32 = 30.0;

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The branch you are on: nothing to do.
    Stay,
    /// Switch to this local branch, by its short name.
    Switch(Vec<u8>),
    /// Review a new local branch, `name`, that starts from and tracks `upstream`
    /// (a `refs/remotes/` ref) no local branch tracks yet.
    Track { upstream: Vec<u8>, name: String },
}

/// One listed branch.
#[derive(Debug)]
pub(crate) struct Row<'a> {
    pub branch: &'a Branch,
    /// The short name shown: `feature/x`, or `origin/feature/x` for a remote.
    pub name: String,
    /// The byte range of the filter's match in `name`, when it can be highlighted.
    pub hit: Option<(usize, usize)>,
    pub pick: Pick,
}

/// What the switcher lists for a filter.
#[derive(Debug, Default)]
pub(crate) struct Listing<'a> {
    /// The checked-out branch, while it matches.
    pub current: Option<Row<'a>>,
    /// The other local branches that match, newest first.
    pub local: Vec<Row<'a>>,
    /// The remote-tracking branches that match, newest first; `origin/HEAD` is left out.
    pub remote: Vec<Row<'a>>,
    /// Every remote-tracking branch, for the folded row's count.
    pub remote_total: usize,
}

/// The rows for `filter`, which matches any part of a short name, ignoring case.
/// `remotes` are the remote names: `origin/feature/x` suggests the local name `feature/x`.
pub(crate) fn listing<'a>(branches: &'a [Branch], remotes: &[Vec<u8>], filter: &str) -> Listing<'a> {
    let needle = filter.trim().to_lowercase();
    let matched = |name: &str| -> Option<Option<(usize, usize)>> {
        if needle.is_empty() {
            return Some(None);
        }
        let at = name.to_lowercase().find(&needle)?;
        let end = at + needle.len();
        // Highlight only where lowercasing kept the name's byte layout.
        let exact = name.get(at..end).is_some_and(|s| s.to_lowercase() == needle);
        Some(exact.then_some((at, end)))
    };
    // The local branch that tracks each remote-tracking branch, looked up once. When
    // several do, the checked-out one wins, so its upstream's row stays where you are;
    // otherwise the first by name.
    let mut tracking: HashMap<&[u8], &Branch> = HashMap::new();
    for local in branches.iter().filter(|b| b.kind == BranchKind::Local) {
        if let Some(upstream) = local.upstream.as_deref() {
            let slot = tracking.entry(upstream).or_insert(local);
            if local.is_head {
                *slot = local;
            }
        }
    }
    let mut listing = Listing::default();
    for branch in branches {
        let (short, remote) = match branch.kind {
            BranchKind::Local => (branch.reference.strip_prefix(b"refs/heads/"), false),
            BranchKind::Remote => (branch.reference.strip_prefix(b"refs/remotes/"), true),
        };
        let Some(short) = short else { continue };
        if remote {
            if branch.symbolic_target.is_some() {
                continue;
            }
            listing.remote_total += 1;
        }
        let name = display(short);
        let Some(hit) = matched(&name) else { continue };
        let pick = if !remote {
            if branch.is_head { Pick::Stay } else { Pick::Switch(short.to_vec()) }
        } else {
            match tracking.get(branch.reference.as_slice()) {
                Some(local) if local.is_head => Pick::Stay,
                Some(local) => Pick::Switch(
                    local.reference.strip_prefix(b"refs/heads/").unwrap_or(&local.reference).to_vec(),
                ),
                None => Pick::Track {
                    upstream: branch.reference.clone(),
                    name: display(local_name(short, remotes)),
                },
            }
        };
        let row = Row { branch, name, hit, pick };
        if remote {
            listing.remote.push(row);
        } else if branch.is_head {
            listing.current = Some(row);
        } else {
            listing.local.push(row);
        }
    }
    // Newest first; a branch Git gave no date sorts last, then by name.
    let newest = |a: &Row, b: &Row| {
        b.branch.committed.cmp(&a.branch.committed).then_with(|| a.name.cmp(&b.name))
    };
    listing.local.sort_by(newest);
    listing.remote.sort_by(newest);
    listing
}

/// The local name a remote-tracking branch suggests: `origin/feature/x` → `feature/x`.
/// The longest remote name that prefixes it wins, as remote names may hold slashes.
fn local_name<'a>(short: &'a [u8], remotes: &[Vec<u8>]) -> &'a [u8] {
    let remote = remotes
        .iter()
        .filter(|r| short.len() > r.len() + 1 && short.starts_with(r) && short[r.len()] == b'/')
        .max_by_key(|r| r.len());
    match remote {
        Some(r) => &short[r.len() + 1..],
        None => short.splitn(2, |&b| b == b'/').nth(1).unwrap_or(short),
    }
}

/// "↑2", "↓3" or "↑1 ↓3": how far a branch is from its upstream; empty when level.
pub(crate) fn distance(ahead_behind: Option<(u32, u32)>) -> String {
    match ahead_behind {
        Some((0, 0)) | None => String::new(),
        Some((a, 0)) => format!("↑{a}"),
        Some((0, b)) => format!("↓{b}"),
        Some((a, b)) => format!("↑{a} ↓{b}"),
    }
}

/// Per-open state; dropped when the list closes, so it opens empty with the filter focused.
#[derive(Clone, Default)]
struct State {
    filter: String,
    /// The highlighted row, by its position among the listed rows.
    highlight: usize,
    /// The remote group, opened from its folded row while nothing is typed.
    remote_open: bool,
}

/// The list under the toolbar's branch button. Choosing a row pushes its intent; with
/// `writable` false (a write running, a preview, a bare repository) rows only show.
/// MCP controls: `branch-switch-filter`, `branch-switch-row-{i}` for the i-th listed row,
/// `branch-switch-remote` (the folded row), `branch-switch-new` and `branch-switch-manage`.
pub(crate) fn popup(
    button: &Response,
    tab: &Tab,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let id = button.id.with("branch-switcher");
    let ctx = button.ctx.clone();
    // Keep the list on screen: what is left below the button, less the field and footer.
    let room = (ctx.content_rect().bottom() - button.rect.bottom() - 120.0).clamp(120.0, 420.0);
    let shown = Popup::menu(button)
        // Clicking into the filter must not close the list.
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(340.0)
        .show(|ui| {
            let saved = ui.data(|d| d.get_temp::<State>(id));
            let opening = saved.is_none();
            let mut state = saved.unwrap_or_default();
            // Before the field draws, so the arrows and Enter move the list, not the cursor.
            let (down, up, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                )
            });
            ui.spacing_mut().item_spacing.y = 2.0;
            let width = ui.available_width();
            let field = pages::search_box(ui, &mut state.filter, "branch-switch-filter", "Find a branch", width);
            pages::control(widgets, "branch-switch-filter", &field, ui.clip_rect());
            if opening {
                field.request_focus();
            }
            ui.add_space(2.0);
            let snapshot = tab.snapshot.as_ref();
            let branches = snapshot.map_or(&[][..], |s| s.branches.as_slice());
            let remotes: Vec<Vec<u8>> =
                snapshot.map_or_else(Vec::new, |s| s.remotes.iter().map(|r| r.name.clone()).collect());
            let typing = !state.filter.trim().is_empty();
            let list = listing(branches, &remotes, &state.filter);
            let remote_shown = typing || state.remote_open;
            // The listed rows in order, for the arrows and Enter.
            let mut rows: Vec<&Row> = list.current.iter().collect();
            rows.extend(list.local.iter().take(SHOWN));
            if remote_shown {
                rows.extend(list.remote.iter().take(SHOWN));
            }
            if opening || field.changed() {
                // Start on the first row that does something, not the current branch.
                state.highlight = rows.iter().position(|r| r.pick != Pick::Stay).unwrap_or(0);
            }
            if !rows.is_empty() {
                state.highlight = state.highlight.min(rows.len() - 1);
                let n = rows.len();
                if down {
                    state.highlight = (state.highlight + 1) % n;
                } else if up {
                    state.highlight = (state.highlight + n - 1) % n;
                }
            }
            let mut chosen = (enter && !rows.is_empty()).then_some(state.highlight);
            // The pointer highlights only when it moves, so a resting mouse keeps the arrows' row.
            let pointer_moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO);
            let mut toggle_remote = false;
            ScrollArea::vertical()
                .id_salt(id.with("rows"))
                .max_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut index = 0;
                    let mut row_ui = |ui: &mut Ui,
                                      row: &Row,
                                      widgets: &mut HashMap<String, UiControl>,
                                      chosen: &mut Option<usize>,
                                      highlight: &mut usize| {
                        let on = index == *highlight;
                        let enabled = writable || row.pick == Pick::Stay;
                        let r = draw_row(ui, row, on, enabled);
                        pages::control(widgets, format!("branch-switch-row-{index}"), &r, ui.clip_rect());
                        if on && (down || up) {
                            r.scroll_to_me(None);
                        }
                        if enabled && r.hovered() && pointer_moved {
                            *highlight = index;
                        }
                        if r.clicked() {
                            *chosen = Some(index);
                        }
                        index += 1;
                    };
                    match &list.current {
                        Some(row) => row_ui(ui, row, widgets, &mut chosen, &mut state.highlight),
                        // Detached or unborn: say where you are, with nothing to choose.
                        None if !typing => head_row(ui, &tab.branch_label()),
                        None => {}
                    }
                    if !list.local.is_empty() {
                        group(ui, &format!("LOCAL · {}", list.local.len()));
                        for row in list.local.iter().take(SHOWN) {
                            row_ui(ui, row, widgets, &mut chosen, &mut state.highlight);
                        }
                        more(ui, list.local.len());
                    }
                    if typing {
                        if !list.remote.is_empty() {
                            group(ui, &format!("REMOTE · {}", list.remote.len()));
                        }
                    } else if list.remote_total > 0 {
                        let r = fold_row(ui, list.remote_total, state.remote_open);
                        pages::control(widgets, "branch-switch-remote", &r, ui.clip_rect());
                        toggle_remote = r.clicked();
                    }
                    if remote_shown {
                        for row in list.remote.iter().take(SHOWN) {
                            row_ui(ui, row, widgets, &mut chosen, &mut state.highlight);
                        }
                        more(ui, list.remote.len());
                    }
                    if typing && rows.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("No branch matches “{}”.", state.filter.trim()))
                                .size(12.5)
                                .color(FAINT()),
                        );
                        ui.add_space(6.0);
                    }
                });
            if toggle_remote {
                state.remote_open = !state.remote_open;
                // The click took focus from the filter; typing should still narrow the list.
                field.request_focus();
            }
            let hint = match rows.get(state.highlight).map(|r| &r.pick) {
                _ if !typing => "↑ ↓ Enter",
                Some(Pick::Track { .. }) => "Enter reviews",
                Some(Pick::Switch(_)) => "Enter switches",
                _ => "",
            };
            footer(ui, writable, hint, intents, widgets);
            if let Some(index) = chosen
                && let Some(row) = rows.get(index)
            {
                match &row.pick {
                    Pick::Stay => {}
                    _ if !writable => return state,
                    Pick::Switch(name) => intents.push(Intent::Execute(tab.id, Action::SwitchBranch(name.clone()))),
                    Pick::Track { upstream, name } => intents.push(Intent::Dialog(DialogKind::TrackBranch {
                        upstream: upstream.clone(),
                        name: name.clone(),
                    })),
                }
                ui.close();
            }
            state
        });
    ctx.data_mut(|d| match shown {
        Some(state) => {
            d.insert_temp(id, state.inner);
        }
        None => {
            d.remove::<State>(id);
        }
    });
}

/// One branch: a check for the current branch, the name with the filter's match
/// highlighted, then its distance from upstream and its age, tag or what choosing does.
fn draw_row(ui: &mut Ui, row: &Row, on: bool, enabled: bool) -> Response {
    let (rect, response) = ui
        .scope(|ui| {
            if !enabled {
                ui.disable();
            }
            ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click())
        })
        .inner;
    let painter = ui.painter_at(rect);
    if on && enabled {
        painter.rect_filled(rect, 5.0, SELECTED());
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 5.0, HOVER());
    }
    let current = row.branch.kind == BranchKind::Local && row.branch.is_head;
    let y = rect.center().y;
    if current {
        icon(&painter, Rect::from_center_size(pos2(rect.left() + 15.0, y), vec2(14.0, 14.0)), Icon::Check, BLUE());
    }
    // Right side first: the tag or detail, then the distance from upstream.
    let mut right = rect.right() - 8.0;
    let detail = match (&row.pick, row.branch.kind) {
        (_, BranchKind::Local) if current => String::new(),
        (_, BranchKind::Local) => row.branch.committed.map(crate::tags::age).unwrap_or_default(),
        (Pick::Track { .. }, _) => "Check out as local".into(),
        (Pick::Switch(_), _) => "tracked".into(),
        (Pick::Stay, _) => "checked out".into(),
    };
    if current {
        right = tag(&painter, right, y, "current") - 8.0;
    } else if !detail.is_empty() {
        let galley = painter.layout_no_wrap(detail.clone(), FontId::proportional(12.0), FAINT());
        right -= galley.size().x;
        painter.galley(pos2(right, y - galley.size().y / 2.0), galley, FAINT());
        right -= 10.0;
    }
    let track = if row.branch.kind == BranchKind::Local { distance(row.branch.ahead_behind) } else { String::new() };
    if !track.is_empty() {
        let galley = painter.layout_no_wrap(track.clone(), FontId::proportional(12.0), ACCENT_TEXT());
        right -= galley.size().x;
        painter.galley(pos2(right, y - galley.size().y / 2.0), galley, ACCENT_TEXT());
        right -= 10.0;
    }
    let ink = if enabled { TEXT() } else { FAINT() };
    let plain = if current { semibold(13.0) } else { FontId::proportional(13.0) };
    let format = |font: FontId, color: Color32| TextFormat { font_id: font, color, ..Default::default() };
    let mut job = LayoutJob::default();
    match row.hit {
        Some((start, end)) => {
            job.append(&row.name[..start], 0.0, format(plain.clone(), ink));
            job.append(&row.name[start..end], 0.0, format(semibold(13.0), if enabled { BLUE() } else { ink }));
            job.append(&row.name[end..], 0.0, format(plain, ink));
        }
        None => job.append(&row.name, 0.0, format(plain, ink)),
    }
    let left = rect.left() + 30.0;
    job.wrap = text::TextWrapping::truncate_at_width((right - left).max(0.0));
    let name = painter.layout_job(job);
    painter.galley(pos2(left, y - name.size().y / 2.0), name, ink);
    let spoken = [row.name.as_str(), if current { "current" } else { detail.as_str() }, track.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &spoken));
    response
}

/// Where HEAD is when no branch is checked out (detached or no commits yet).
fn head_row(ui: &mut Ui, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let painter = ui.painter_at(rect);
    let right = tag(&painter, rect.right() - 8.0, rect.center().y, "current") - 8.0;
    let mut job = LayoutJob::simple_singleline(label.to_owned(), semibold(13.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width((right - rect.left() - 30.0).max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
}

/// A small outlined tag ending at `right`; returns its left edge.
fn tag(painter: &Painter, right: f32, y: f32, text: &str) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), semibold(11.0), BLUE());
    let rect = Rect::from_min_max(pos2(right - galley.size().x - 14.0, y - 9.0), pos2(right, y + 9.0));
    painter.rect_stroke(rect, 9.0, Stroke::new(1.0, BLUE().gamma_multiply(0.45)), StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, BLUE());
    rect.left()
}

/// A group's title, such as "LOCAL · 3".
fn group(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let mut job = LayoutJob::default();
    job.append(
        title,
        0.0,
        TextFormat { font_id: semibold(11.0), color: FAINT(), extra_letter_spacing: 0.6, ..Default::default() },
    );
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley, FAINT());
}

/// How many more a group has than it lists.
fn more(ui: &mut Ui, total: usize) {
    if total > SHOWN {
        ui.label(
            RichText::new(format!("{} more; type to narrow the list.", total - SHOWN))
                .size(12.0)
                .color(FAINT()),
        );
    }
}

/// The remote branches' folded row; clicking it opens or folds the group.
fn fold_row(ui: &mut Ui, count: usize, open: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let painter = ui.painter_at(rect);
    if response.hovered() {
        painter.rect_filled(rect, 5.0, HOVER());
    }
    let chevron = if open { Icon::Chevron } else { Icon::ChevronRight };
    icon(&painter, Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(12.0, 12.0)), chevron, MUTED());
    let label = format!("Remote branches · {count}");
    painter.text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, &label, FontId::proportional(13.0), MUTED());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    response
}

/// New branch and Manage branches, then what the keys do.
fn footer(ui: &mut Ui, writable: bool, hint: &str, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    ui.add_space(2.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, HAIRLINE());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding = vec2(11.0, 4.0);
        // Menus draw idle buttons without a frame; this one keeps its frame, as on the board.
        let new = ui.add_enabled(
            writable,
            Button::new("New branch").fill(RAISED()).stroke(Stroke::new(1.0, BORDER())).min_size(vec2(0.0, 28.0)),
        );
        pages::control(widgets, "branch-switch-new", &new, ui.clip_rect());
        if new.clicked() {
            intents.push(Intent::Dialog(DialogKind::NewBranch));
            ui.close();
        }
        let manage = ui.add(Button::new(RichText::new("Manage branches").color(MUTED())).frame(false).min_size(vec2(0.0, 28.0)));
        pages::control(widgets, "branch-switch-manage", &manage, ui.clip_rect());
        if manage.clicked() {
            intents.push(Intent::View(View::Branches));
            ui.close();
        }
        if !hint.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(hint).size(11.5).color(FAINT()));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket_git::ObjectId;

    fn branch(reference: &str, committed: Option<i64>) -> Branch {
        Branch {
            reference: reference.as_bytes().to_vec(),
            target: ObjectId::parse("1111111111111111111111111111111111111111").unwrap(),
            kind: if reference.starts_with("refs/heads/") { BranchKind::Local } else { BranchKind::Remote },
            is_head: false,
            upstream: None,
            symbolic_target: None,
            checked_out_at: None,
            committed,
            ahead_behind: None,
        }
    }

    fn names<'a>(rows: &[Row<'a>]) -> Vec<&'a str> {
        rows.iter().map(|r| std::str::from_utf8(r.branch.reference.as_slice()).unwrap()).collect()
    }

    fn fixture() -> Vec<Branch> {
        let mut current = branch("refs/heads/main", Some(500));
        current.is_head = true;
        current.upstream = Some(b"refs/remotes/origin/main".to_vec());
        current.ahead_behind = Some((2, 0));
        let mut settings = branch("refs/heads/feature/retry-settings", Some(300));
        settings.upstream = Some(b"refs/remotes/origin/feature/retry-settings".to_vec());
        let mut head = branch("refs/remotes/origin/HEAD", None);
        head.symbolic_target = Some(b"refs/remotes/origin/main".to_vec());
        vec![
            branch("refs/heads/docs/settings-page", Some(400)),
            settings,
            branch("refs/heads/fix/backoff-jitter", Some(100)),
            branch("refs/heads/undated", None),
            current,
            head,
            branch("refs/remotes/origin/main", Some(450)),
            branch("refs/remotes/origin/feature/retry-settings", Some(300)),
            branch("refs/remotes/origin/retry-limit", Some(350)),
            branch("refs/remotes/my/fork/retry-docs", Some(200)),
        ]
    }

    #[test]
    fn the_current_branch_comes_first_then_local_branches_newest_first() {
        let branches = fixture();
        let list = listing(&branches, &[b"origin".to_vec(), b"my/fork".to_vec()], "");
        let current = list.current.as_ref().unwrap();
        assert_eq!(current.name, "main");
        assert_eq!(current.pick, Pick::Stay);
        assert_eq!(
            names(&list.local),
            ["refs/heads/docs/settings-page", "refs/heads/feature/retry-settings", "refs/heads/fix/backoff-jitter", "refs/heads/undated"],
            "newest first, an undated branch last"
        );
        assert_eq!(list.local[0].pick, Pick::Switch(b"docs/settings-page".to_vec()));
        // origin/HEAD is a pointer, not a branch to check out.
        assert_eq!(list.remote_total, 4);
        assert_eq!(list.remote.len(), 4);
    }

    #[test]
    fn typing_narrows_local_and_remote_branches_and_marks_the_match() {
        let branches = fixture();
        let list = listing(&branches, &[b"origin".to_vec(), b"my/fork".to_vec()], " RETRY ");
        assert!(list.current.is_none(), "main does not match");
        assert_eq!(names(&list.local), ["refs/heads/feature/retry-settings"]);
        assert_eq!(list.local[0].hit, Some((8, 13)));
        assert_eq!(
            names(&list.remote),
            ["refs/remotes/origin/retry-limit", "refs/remotes/origin/feature/retry-settings", "refs/remotes/my/fork/retry-docs"]
        );
        // No local branch tracks it: a review of a new branch that does.
        assert_eq!(
            list.remote[0].pick,
            Pick::Track { upstream: b"refs/remotes/origin/retry-limit".to_vec(), name: "retry-limit".into() }
        );
        // A local branch tracks it: switch to that branch.
        assert_eq!(list.remote[1].pick, Pick::Switch(b"feature/retry-settings".to_vec()));
        // The longest remote name wins, so a remote named with a slash keeps its branch name.
        assert_eq!(
            list.remote[2].pick,
            Pick::Track { upstream: b"refs/remotes/my/fork/retry-docs".to_vec(), name: "retry-docs".into() }
        );
        let list = listing(&branches, &[b"origin".to_vec()], "origin/main");
        assert_eq!(list.remote[0].pick, Pick::Stay, "the current branch tracks it");
        assert!(listing(&branches, &[], "release").local.is_empty());
    }

    /// From the toolbar: the filter has focus, ↓ moves the highlight, Enter switches to
    /// it and closes the list, and Esc closes it without switching.
    #[test]
    fn the_toolbar_list_types_into_its_filter_moves_with_the_arrows_and_switches_on_enter() {
        use crate::{
            batch::flow_tests::{click, frame, open, wait},
            test_common as common,
        };
        common::isolated(
            "ROCKET_BRANCH_SWITCHER_UI_CASE",
            "branch_switcher::tests::the_toolbar_list_types_into_its_filter_moves_with_the_arrows_and_switches_on_enter",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                for name in ["feature/retry-a", "feature/retry-b", "docs/page"] {
                    f.run(&["branch", name]);
                }
                // One remote branch, as a fetch leaves it, so the folded row shows.
                f.run(&["remote", "add", "origin", "https://example.invalid/rocket.git"]);
                f.run(&["update-ref", "refs/remotes/origin/docs/old-page", "HEAD"]);
                let current = || String::from_utf8(f.run(&["branch", "--show-current"])).unwrap().trim().to_owned();
                let home = current();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let key = |key: Key, pressed: bool| Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                };
                click(&ctx, &mut app, "branch-switch");
                assert!(app.frame_widgets.contains_key("branch-switch-filter"), "the list opens");
                frame(&ctx, &mut app, vec![key(Key::Escape, true), key(Key::Escape, false)]);
                frame(&ctx, &mut app, vec![]);
                assert!(!app.frame_widgets.contains_key("branch-switch-filter"), "Esc closes it");
                assert_eq!(current(), home);
                click(&ctx, &mut app, "branch-switch");
                // Opening the remote group keeps the filter focused for typing.
                click(&ctx, &mut app, "branch-switch-remote");
                assert!(app.frame_widgets.contains_key("branch-switch-row-4"), "current, 3 local and 1 remote row");
                frame(&ctx, &mut app, vec![Event::Text("retry".into())]);
                frame(&ctx, &mut app, vec![]);
                // Two matches, same date, so by name; the highlight starts on the first.
                assert!(app.frame_widgets.contains_key("branch-switch-row-1"));
                assert!(!app.frame_widgets.contains_key("branch-switch-row-2"), "only the matches are listed");
                frame(&ctx, &mut app, vec![key(Key::ArrowDown, true), key(Key::ArrowDown, false)]);
                frame(&ctx, &mut app, vec![key(Key::Enter, true), key(Key::Enter, false)]);
                wait(&ctx, &mut app);
                assert_eq!(current(), "feature/retry-b");
                assert!(!app.frame_widgets.contains_key("branch-switch-filter"), "choosing closes the list");
            },
        );
    }

    #[test]
    fn a_remote_branch_several_local_branches_track_prefers_the_checked_out_one() {
        let tracking = |name: &str, head: bool| {
            let mut b = branch(name, Some(1));
            b.upstream = Some(b"refs/remotes/origin/main".to_vec());
            b.is_head = head;
            b
        };
        let remote = branch("refs/remotes/origin/main", Some(1));
        // main-old sorts after main, as Git lists them.
        let branches = [tracking("refs/heads/main", true), tracking("refs/heads/main-old", false), remote.clone()];
        assert_eq!(listing(&branches, &[], "origin/main").remote[0].pick, Pick::Stay);
        let branches = [tracking("refs/heads/main", false), tracking("refs/heads/main-old", true), remote.clone()];
        assert_eq!(listing(&branches, &[], "origin/main").remote[0].pick, Pick::Stay);
        // Neither checked out: the first by name.
        let branches = [tracking("refs/heads/main", false), tracking("refs/heads/main-old", false), remote];
        assert_eq!(listing(&branches, &[], "origin/main").remote[0].pick, Pick::Switch(b"main".to_vec()));
    }

    #[test]
    fn the_distance_from_upstream_leaves_out_what_is_level() {
        assert_eq!(distance(None), "");
        assert_eq!(distance(Some((0, 0))), "");
        assert_eq!(distance(Some((2, 0))), "↑2");
        assert_eq!(distance(Some((0, 3))), "↓3");
        assert_eq!(distance(Some((1, 3))), "↑1 ↓3");
    }
}
