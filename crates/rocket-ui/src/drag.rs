//! Drag and drop. A branch dropped on a branch, or a commit dropped on a commit,
//! opens a small menu; each choice opens the existing reviewed merge, rebase or
//! interactive-rebase dialog, prefilled. Nothing changes until that review is
//! applied.
use crate::{app::*, pages, style::*};
use eframe::egui::*;
use rocket_git::*;
use std::collections::HashMap;

/// Most commits an interactive rebase plan holds.
const PLAN_LIMIT: usize = 200;

/// A branch dragged from the sidebar.
struct BranchDrag {
    tab: u64,
    reference: Vec<u8>,
}
/// A commit dragged in the current branch's history.
struct CommitDrag {
    tab: u64,
    id: ObjectId,
}

/// A drop waiting for the user's choice.
#[derive(Clone, Debug)]
pub struct PendingDrop {
    pub tab: u64,
    pub pos: Pos2,
    pub kind: DropKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropKind {
    /// Full ref names.
    Branch {
        source: Vec<u8>,
        target: Vec<u8>,
    },
    Commit {
        source: ObjectId,
        target: ObjectId,
    },
}

/// Where a dropped commit goes, relative to the commit it was dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitChoice {
    /// Directly newer than the target (above it in the history list).
    Above,
    /// Directly older than the target (below it).
    Below,
    /// Combine into the target, keeping both messages for editing.
    Squash,
    /// Combine into the target, keeping only the target's message.
    Fixup,
}

/// An interactive rebase plan prepared by a commit drop: every commit after
/// `base`, oldest first; with no base, every commit from the root commit.
#[derive(Clone, Debug)]
pub struct Arrangement {
    pub base: Option<ObjectId>,
    pub entries: Vec<RebasePlanEntry>,
}

/// The current branch's history an interactive rebase can rewrite: from HEAD
/// along first parents, stopping before a merge commit. Newest first.
fn linear<'a>(commits: &'a [Commit], head: &ObjectId) -> Vec<&'a Commit> {
    let by_id: HashMap<&ObjectId, &Commit> = commits.iter().map(|c| (&c.id, c)).collect();
    let mut line = Vec::new();
    let mut next = Some(head);
    while let Some(id) = next {
        let Some(commit) = by_id.get(id) else { break };
        if commit.parents.len() > 1 {
            break;
        }
        line.push(*commit);
        next = commit.parents.first();
    }
    line
}

fn message(commit: &Commit) -> String {
    String::from_utf8_lossy(&commit.message)
        .trim_end()
        .to_owned()
}

/// The plan that puts `source` where `choice` says, relative to `target`.
pub fn arrange(
    commits: &[Commit],
    head: &ObjectId,
    source: &ObjectId,
    target: &ObjectId,
    choice: CommitChoice,
) -> std::result::Result<Arrangement, &'static str> {
    if source == target {
        return Err("Drop the commit on a different commit.");
    }
    let line = linear(commits, head);
    let position = |id: &ObjectId| line.iter().position(|c| &c.id == id);
    let (Some(s), Some(t)) = (position(source), position(target)) else {
        return Err("Only commits of the current branch after its latest merge can be rearranged.");
    };
    let target_commit = line[t];
    // Newest first, so the larger index is the older commit.
    let oldest = s.max(t);
    if oldest >= PLAN_LIMIT {
        return Err("An interactive rebase plan holds at most 200 commits.");
    }
    // Without a parent the oldest commit is the root, so the plan holds the whole branch.
    let base = line[oldest].parents.first().cloned();
    let original: Vec<&Commit> = line[..=oldest].iter().rev().copied().collect();
    let mut order = original.clone();
    let moved = order.remove(order.iter().position(|c| &c.id == source).unwrap());
    let at = order.iter().position(|c| &c.id == target).unwrap();
    order.insert(
        if choice == CommitChoice::Below {
            at
        } else {
            at + 1
        },
        moved,
    );
    if matches!(choice, CommitChoice::Above | CommitChoice::Below)
        && order
            .iter()
            .map(|c| &c.id)
            .eq(original.iter().map(|c| &c.id))
    {
        return Err("The commit is already there.");
    }
    let entries = order
        .iter()
        .map(|c| {
            let mut entry = RebasePlanEntry::new(c.id.clone(), RebaseInstruction::Pick);
            if &c.id == source {
                match choice {
                    CommitChoice::Squash => {
                        entry.instruction = RebaseInstruction::Squash;
                        entry.message = format!("{}\n\n{}", message(target_commit), message(c));
                    }
                    CommitChoice::Fixup => entry.instruction = RebaseInstruction::Fixup,
                    CommitChoice::Above | CommitChoice::Below => (),
                }
            }
            entry
        })
        .collect();
    Ok(Arrangement { base, entries })
}

/// What dropping branch `source` on branch `target` can do. Merge and rebase change
/// the checked-out branch, so one of the two must be it. Returns the current
/// branch's name and the other branch, or why nothing applies.
pub fn branch_choices<'a>(
    branches: &'a [Branch],
    source: &'a [u8],
    target: &'a [u8],
) -> std::result::Result<(&'a Branch, &'a [u8]), String> {
    let current = branches
        .iter()
        .find(|b| b.is_head && b.kind == BranchKind::Local)
        .ok_or("Merge and rebase need a checked-out branch.")?;
    if source == target {
        Err("Drop the branch on a different branch.".into())
    } else if target == current.reference {
        Ok((current, source))
    } else if source == current.reference {
        Ok((current, target))
    } else {
        Err(format!(
            "Merge and rebase change the checked-out branch, {}. Drag a branch onto it, or drag it onto another branch.",
            short(&current.reference)
        ))
    }
}

fn short(reference: &[u8]) -> String {
    let name = reference
        .strip_prefix(b"refs/heads/")
        .or_else(|| reference.strip_prefix(b"refs/remotes/"))
        .unwrap_or(reference);
    display(name)
}

/// A floating label next to the pointer while something is dragged.
fn drag_label(ui: &Ui, text: &str) {
    let Some(pos) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, Id::new("rocket-drag-label")));
    let galley = painter.layout_no_wrap(text.into(), FontId::proportional(12.5), TEXT());
    let rect = Rect::from_min_size(pos + vec2(14.0, 10.0), galley.size() + vec2(16.0, 10.0));
    painter.rect(
        rect,
        6.0,
        RAISED(),
        Stroke::new(1.0, BLUE()),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(8.0, 5.0), galley, TEXT());
}

fn drop_highlight(ui: &Ui, rect: Rect) {
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        5.0,
        Stroke::new(1.5, BLUE()),
        StrokeKind::Inside,
    );
}

/// Make a sidebar branch row draggable; a local branch row also accepts a drop.
/// The row is published as `sidebar-branch-<full ref>`.
pub(crate) fn branch(
    ui: &Ui,
    row: Response,
    tab: u64,
    branch: &Branch,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) -> Response {
    let row = row.interact(Sense::drag());
    pages::control(
        widgets,
        format!("sidebar-branch-{}", display(&branch.reference)),
        &row,
        ui.clip_rect(),
    );
    row.dnd_set_drag_payload(BranchDrag {
        tab,
        reference: branch.reference.clone(),
    });
    if row.dragged() {
        drag_label(ui, &short(&branch.reference));
    }
    if branch.kind == BranchKind::Local && branch.symbolic_target.is_none() {
        let accepts = |d: &BranchDrag| d.tab == tab && d.reference != branch.reference;
        if row
            .dnd_hover_payload::<BranchDrag>()
            .is_some_and(|d| accepts(&d))
        {
            drop_highlight(ui, row.rect);
        }
        if let Some(d) = row.dnd_release_payload::<BranchDrag>()
            && accepts(&d)
        {
            intents.push(Intent::Drop(PendingDrop {
                tab,
                pos: ui.ctx().pointer_latest_pos().unwrap_or(row.rect.center()),
                kind: DropKind::Branch {
                    source: d.reference.clone(),
                    target: branch.reference.clone(),
                },
            }));
        }
    }
    row
}

/// Make a commit row of the current branch's history draggable and a drop target.
/// Other histories, searches and detached HEADs keep plain rows.
pub(crate) fn commit(
    ui: &Ui,
    row: Response,
    tab: &Tab,
    commit: &Commit,
    intents: &mut Vec<Intent>,
) -> Response {
    let on_branch = tab
        .snapshot
        .as_ref()
        .and_then(|s| s.head.as_ref())
        .is_some_and(|h| matches!(h, Head::Branch { .. }));
    if !on_branch || tab.history_all || tab.history_ref.is_some() || !tab.search.trim().is_empty() {
        return row;
    }
    let row = row.interact(Sense::drag());
    row.dnd_set_drag_payload(CommitDrag {
        tab: tab.id,
        id: commit.id.clone(),
    });
    if row.dragged() {
        drag_label(ui, &subject(commit));
    }
    let accepts = |d: &CommitDrag| d.tab == tab.id && d.id != commit.id;
    if row
        .dnd_hover_payload::<CommitDrag>()
        .is_some_and(|d| accepts(&d))
    {
        drop_highlight(ui, row.rect);
    }
    if let Some(d) = row.dnd_release_payload::<CommitDrag>()
        && accepts(&d)
    {
        intents.push(Intent::Drop(PendingDrop {
            tab: tab.id,
            pos: ui.ctx().pointer_latest_pos().unwrap_or(row.rect.center()),
            kind: DropKind::Commit {
                source: d.id.clone(),
                target: commit.id.clone(),
            },
        }));
    }
    row
}

impl Rocket {
    /// The menu of a pending drop. Each choice opens a reviewed dialog; Cancel,
    /// Escape or a click elsewhere discards the drop.
    pub(crate) fn drop_menu(&mut self, ctx: &Context) {
        let Some(pending) = self.pending_drop.clone() else {
            return;
        };
        let writable = self.can_write();
        // Field access, so the menu can register its controls meanwhile.
        let Some(tab) = self
            .tabs
            .get(self.active)
            .filter(|t| t.id == pending.tab && self.dialog.is_none())
        else {
            self.pending_drop = None;
            return;
        };
        let snapshot = tab.snapshot.as_ref();
        let mut chosen = None;
        let mut close = false;
        let widgets = &mut self.frame_widgets;
        // The pointer rests on the menu's heading, not a choice. egui hit-tests rows
        // behind a popup until it covers the pointer's whole search radius, and a
        // hovered row's tooltip would be drawn over the choices.
        let area = Area::new(Id::new("rocket-drop-menu"))
            .order(Order::Foreground)
            .fixed_pos(pending.pos - vec2(16.0, 14.0))
            .constrain(true)
            .show(ctx, |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(360.0);
                    let mut item = |ui: &mut Ui, key: &str, label: &str, choice: Option<Intent>| {
                        let r = ui.add_enabled(writable && choice.is_some(), Button::new(label));
                        pages::control(widgets, key, &r, ui.clip_rect());
                        if r.clicked() {
                            chosen = choice;
                        }
                    };
                    match &pending.kind {
                        DropKind::Branch { source, target } => {
                            ui.label(
                                RichText::new(format!(
                                    "Drop “{}” on “{}”",
                                    short(source),
                                    short(target)
                                ))
                                .strong(),
                            );
                            let branches = snapshot.map_or(&[][..], |s| s.branches.as_slice());
                            match branch_choices(branches, source, target) {
                                Ok((current, other)) => {
                                    let (current_name, other_name) =
                                        (short(&current.reference), short(other));
                                    let reference = String::from_utf8(other.to_vec()).ok();
                                    item(
                                        ui,
                                        "drop-merge",
                                        &format!("Merge {other_name} into {current_name}"),
                                        reference.clone().map(Intent::MergeBranch),
                                    );
                                    item(
                                        ui,
                                        "drop-rebase",
                                        &format!("Rebase {current_name} on top of {other_name}"),
                                        reference.map(Intent::RebaseOnto),
                                    );
                                }
                                Err(reason) => pages::note(ui, &reason),
                            }
                        }
                        DropKind::Commit { source, target } => {
                            let commits = tab.commits();
                            let head =
                                snapshot
                                    .and_then(|s| s.head.as_ref())
                                    .and_then(|h| match h {
                                        Head::Branch { target, .. } => Some(target),
                                        _ => None,
                                    });
                            let label = |id: &ObjectId| {
                                commits
                                    .iter()
                                    .find(|c| &c.id == id)
                                    .map_or_else(|| id.to_string()[..9].to_owned(), subject)
                            };
                            let (source_label, target_label) = (label(source), label(target));
                            ui.label(RichText::new(format!("Move “{source_label}”")).strong());
                            let mut reason = None;
                            for (key, choice, text) in [
                                (
                                    "drop-above",
                                    CommitChoice::Above,
                                    format!("Above “{target_label}”"),
                                ),
                                (
                                    "drop-below",
                                    CommitChoice::Below,
                                    format!("Below “{target_label}”"),
                                ),
                                (
                                    "drop-squash",
                                    CommitChoice::Squash,
                                    format!("Squash into “{target_label}”"),
                                ),
                                (
                                    "drop-fixup",
                                    CommitChoice::Fixup,
                                    format!("Fixup into “{target_label}” (drop its message)"),
                                ),
                            ] {
                                let plan = head
                                    .ok_or("Rearranging commits needs a checked-out branch.")
                                    .and_then(|head| {
                                        arrange(commits, head, source, target, choice)
                                    });
                                if let Err(why) = &plan {
                                    reason.get_or_insert(*why);
                                }
                                item(ui, key, &text, plan.ok().map(Intent::ArrangeCommits));
                            }
                            if let Some(reason) = reason {
                                pages::note(ui, reason);
                            }
                        }
                    }
                    ui.separator();
                    let r = ui.button("Cancel");
                    pages::control(widgets, "drop-cancel", &r, ui.clip_rect());
                    close |= r.clicked();
                });
            });
        close |= area.response.clicked_elsewhere() || ctx.input(|i| i.key_pressed(Key::Escape));
        if let Some(intent) = chosen {
            self.pending_drop = None;
            self.dispatch(intent);
        } else if close {
            self.pending_drop = None;
        }
    }

    /// Open the reviewed rebase of the current branch on top of `reference`.
    pub(crate) fn rebase_onto(&mut self, reference: String) {
        self.show_dialog(DialogKind::Rebase(RebaseAction::Start));
        if let Some(dialog) = &mut self.dialog {
            dialog.first = reference;
        }
        self.start_rebase_preview();
    }

    /// Open the interactive rebase review with a plan prepared by a drop.
    pub(crate) fn arrange_commits(&mut self, arrangement: Arrangement) {
        self.show_dialog(DialogKind::Rebase(RebaseAction::Interactive));
        if let Some(dialog) = &mut self.dialog {
            dialog.first = arrangement
                .base
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            dialog.rebase = Some(crate::rebase::DialogState {
                root: arrangement.base.is_none(),
                entries: arrangement.entries,
                ..Default::default()
            });
        }
        self.start_rebase_preview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ObjectId {
        ObjectId::parse(&format!("{n:040x}")).unwrap()
    }
    fn commit(n: u8, parents: &[u8]) -> Commit {
        let who = Signature {
            name: b"Fixture".to_vec(),
            email: b"fixture@example.invalid".to_vec(),
            timestamp: 0,
            offset_minutes: 0,
        };
        Commit {
            id: id(n),
            tree: id(200),
            parents: parents.iter().map(|p| id(*p)).collect(),
            author: who.clone(),
            committer: who,
            encoding: None,
            message: format!("message {n}\n").into_bytes(),
        }
    }
    /// HEAD 4 → 3 → 2 → 1 → 0 (root), newest first as in the history list.
    fn line() -> Vec<Commit> {
        vec![
            commit(4, &[3]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[0]),
            commit(0, &[]),
        ]
    }
    fn plan(a: &Arrangement) -> Vec<(ObjectId, RebaseInstruction, String)> {
        a.entries
            .iter()
            .map(|e| (e.commit.clone(), e.instruction, e.message.clone()))
            .collect()
    }

    #[test]
    fn a_dropped_commit_becomes_an_interactive_rebase_plan_after_the_older_commit() {
        let commits = line();
        let pick = |n| (id(n), RebaseInstruction::Pick, String::new());
        // HEAD below commit 2: everything after commit 1 is replayed, oldest first.
        let below = arrange(&commits, &id(4), &id(4), &id(2), CommitChoice::Below).unwrap();
        assert_eq!(below.base, Some(id(1)));
        assert_eq!(plan(&below), [pick(4), pick(2), pick(3)]);
        // Commit 2 above HEAD; the base follows the older of the two.
        let above = arrange(&commits, &id(4), &id(2), &id(4), CommitChoice::Above).unwrap();
        assert_eq!(above.base, Some(id(1)));
        assert_eq!(plan(&above), [pick(3), pick(4), pick(2)]);
        // Squash keeps both messages for editing; fixup keeps the target's only.
        let squash = arrange(&commits, &id(4), &id(4), &id(2), CommitChoice::Squash).unwrap();
        assert_eq!(
            plan(&squash),
            [
                pick(2),
                (
                    id(4),
                    RebaseInstruction::Squash,
                    "message 2\n\nmessage 4".into()
                ),
                pick(3)
            ]
        );
        let fixup = arrange(&commits, &id(4), &id(1), &id(3), CommitChoice::Fixup).unwrap();
        assert_eq!(fixup.base, Some(id(0)));
        assert_eq!(
            plan(&fixup),
            [
                pick(2),
                pick(3),
                (id(1), RebaseInstruction::Fixup, String::new()),
                pick(4)
            ]
        );
    }

    #[test]
    fn a_drop_that_reaches_the_root_commit_plans_the_whole_branch() {
        let commits = line();
        let pick = |n| (id(n), RebaseInstruction::Pick, String::new());
        // Commit 1 below the root: it becomes the new root, and nothing is a base.
        let below = arrange(&commits, &id(4), &id(1), &id(0), CommitChoice::Below).unwrap();
        assert_eq!(below.base, None);
        assert_eq!(plan(&below), [pick(1), pick(0), pick(2), pick(3), pick(4)]);
        let fixup = arrange(&commits, &id(4), &id(1), &id(0), CommitChoice::Fixup).unwrap();
        assert_eq!(fixup.base, None);
        assert_eq!(
            plan(&fixup),
            [
                pick(0),
                (id(1), RebaseInstruction::Fixup, String::new()),
                pick(2),
                pick(3),
                pick(4)
            ]
        );
    }

    #[test]
    fn drops_that_cannot_become_a_plan_are_refused_with_a_reason() {
        let commits = line();
        let refused = |source, target, choice| {
            arrange(&commits, &id(4), &id(source), &id(target), choice).unwrap_err()
        };
        assert_eq!(
            refused(3, 2, CommitChoice::Above),
            "The commit is already there."
        );
        assert_eq!(
            refused(2, 2, CommitChoice::Squash),
            "Drop the commit on a different commit."
        );
        // Only HEAD's first-parent history after its latest merge can be rewritten.
        let merged = vec![commit(9, &[8]), commit(8, &[4, 7]), commit(7, &[0])]
            .into_iter()
            .chain(line())
            .collect::<Vec<_>>();
        assert!(arrange(&merged, &id(9), &id(9), &id(4), CommitChoice::Below).is_err());
        assert!(arrange(&merged, &id(9), &id(9), &id(8), CommitChoice::Below).is_err());
    }

    #[test]
    fn a_branch_drop_needs_the_current_branch_on_one_side() {
        let branch = |name: &str, head: bool| Branch {
            reference: format!("refs/heads/{name}").into_bytes(),
            target: id(1),
            kind: BranchKind::Local,
            is_head: head,
            upstream: None,
            symbolic_target: None,
            checked_out_at: None,
        };
        let branches = [
            branch("main", true),
            branch("feature", false),
            branch("other", false),
        ];
        let r = |name: &str| format!("refs/heads/{name}").into_bytes();
        let choice = |source: &str, target: &str| {
            branch_choices(&branches, &r(source), &r(target))
                .map(|(current, other)| (short(&current.reference), short(other)))
        };
        let expected = Ok(("main".to_owned(), "feature".to_owned()));
        assert_eq!(choice("feature", "main"), expected);
        assert_eq!(choice("main", "feature"), expected);
        assert!(choice("feature", "other").unwrap_err().contains("main"));
        assert!(choice("main", "main").is_err());
    }
}

#[cfg(test)]
mod flow_tests {
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, drag, enabled, open, wait},
        drag::DropKind,
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::{RebaseAction, RebaseInstruction};

    const CASE: &str = "ROCKET_DRAG_UI_CASE";

    #[test]
    fn dropping_a_branch_on_the_current_branch_opens_a_reviewed_rebase() {
        common::isolated(
            CASE,
            "drag::flow_tests::dropping_a_branch_on_the_current_branch_opens_a_reviewed_rebase",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                f.run(&["branch", "feature"]);
                f.run(&["switch", "--quiet", "feature"]);
                f.write("b.txt", "2\n");
                f.commit("feature work");
                f.run(&["switch", "--quiet", "main"]);
                f.write("c.txt", "3\n");
                f.commit("main work");
                let main = f.run(&["rev-parse", "main"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                drag(
                    &ctx,
                    &mut app,
                    "sidebar-branch-refs/heads/feature",
                    "sidebar-branch-refs/heads/main",
                );
                assert_eq!(
                    app.pending_drop.as_ref().map(|d| d.kind.clone()),
                    Some(DropKind::Branch {
                        source: b"refs/heads/feature".to_vec(),
                        target: b"refs/heads/main".to_vec()
                    })
                );
                assert!(enabled(&ctx, &mut app, "drop-merge"));
                click(&ctx, &mut app, "drop-rebase");
                assert!(app.pending_drop.is_none());
                let dialog = app.dialog.as_ref().unwrap();
                assert!(matches!(
                    dialog.kind,
                    DialogKind::Rebase(RebaseAction::Start)
                ));
                assert_eq!(dialog.first, "refs/heads/feature");
                wait(&ctx, &mut app);
                let preview = app
                    .dialog
                    .as_ref()
                    .unwrap()
                    .rebase
                    .as_ref()
                    .unwrap()
                    .preview
                    .clone();
                assert_eq!(preview.unwrap().commits().len(), 1);
                // Dropping only opened the review.
                assert_eq!(f.run(&["rev-parse", "main"]), main);
                // A drop between two branches that are not checked out offers nothing.
                app.dialog = None;
                f.run(&["branch", "other"]);
                app.dispatch(Intent::Refresh);
                wait(&ctx, &mut app);
                drag(
                    &ctx,
                    &mut app,
                    "sidebar-branch-refs/heads/feature",
                    "sidebar-branch-refs/heads/other",
                );
                // Only the reason and Cancel are offered.
                assert!(app.pending_drop.is_some());
                assert!(enabled(&ctx, &mut app, "drop-cancel"));
                assert!(!app.frame_widgets.contains_key("drop-merge"));
                assert!(!app.frame_widgets.contains_key("drop-rebase"));
                click(&ctx, &mut app, "drop-cancel");
                assert!(app.pending_drop.is_none() && app.dialog.is_none());
            },
        );
    }

    #[test]
    fn dropping_a_commit_opens_the_interactive_rebase_review_with_the_plan() {
        common::isolated(
            CASE,
            "drag::flow_tests::dropping_a_commit_opens_the_interactive_rebase_review_with_the_plan",
            || {
                let f = common::Fixture::new();
                let mut ids = Vec::new();
                for n in 1..=4 {
                    f.write(&format!("{n}.txt"), "x\n");
                    f.commit(&format!("commit {n}"));
                    let id = f.run(&["rev-parse", "HEAD"]);
                    ids.push(String::from_utf8(id).unwrap().trim().to_owned());
                }
                let head = ids[3].clone();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                drag(
                    &ctx,
                    &mut app,
                    &format!("commit-{}", ids[3]),
                    &format!("commit-{}", ids[1]),
                );
                assert!(matches!(
                    app.pending_drop.as_ref().map(|d| &d.kind),
                    Some(DropKind::Commit { .. })
                ));
                assert!(enabled(&ctx, &mut app, "drop-squash"));
                click(&ctx, &mut app, "drop-below");
                let dialog = app.dialog.as_ref().unwrap();
                assert!(matches!(
                    dialog.kind,
                    DialogKind::Rebase(RebaseAction::Interactive)
                ));
                assert_eq!(dialog.first, ids[0]);
                wait(&ctx, &mut app);
                // The loaded review keeps the dropped order: commit 4 now replays first.
                let state = app.dialog.as_ref().unwrap().rebase.as_ref().unwrap();
                assert!(state.preview.is_some());
                let order: Vec<(String, RebaseInstruction)> = state
                    .entries
                    .iter()
                    .map(|e| (e.commit.to_string(), e.instruction))
                    .collect();
                assert_eq!(
                    order,
                    [
                        (ids[3].clone(), RebaseInstruction::Pick),
                        (ids[1].clone(), RebaseInstruction::Pick),
                        (ids[2].clone(), RebaseInstruction::Pick)
                    ]
                );
                assert_eq!(
                    String::from_utf8(f.run(&["rev-parse", "HEAD"]))
                        .unwrap()
                        .trim(),
                    head
                );
            },
        );
    }

    #[test]
    fn dropping_a_commit_below_the_root_commit_opens_a_review_from_the_root() {
        common::isolated(
            CASE,
            "drag::flow_tests::dropping_a_commit_below_the_root_commit_opens_a_review_from_the_root",
            || {
                let f = common::Fixture::new();
                let mut ids = Vec::new();
                for n in 1..=3 {
                    f.write(&format!("{n}.txt"), "x\n");
                    f.commit(&format!("commit {n}"));
                    let id = f.run(&["rev-parse", "HEAD"]);
                    ids.push(String::from_utf8(id).unwrap().trim().to_owned());
                }
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                drag(
                    &ctx,
                    &mut app,
                    &format!("commit-{}", ids[2]),
                    &format!("commit-{}", ids[0]),
                );
                click(&ctx, &mut app, "drop-below");
                let dialog = app.dialog.as_ref().unwrap();
                assert!(dialog.first.is_empty());
                assert!(dialog.rebase.as_ref().unwrap().root);
                wait(&ctx, &mut app);
                let dialog = app.dialog.as_ref().unwrap();
                let state = dialog.rebase.as_ref().unwrap();
                let preview = state
                    .preview
                    .as_ref()
                    .unwrap_or_else(|| panic!("{:?}", dialog.error));
                assert_eq!(preview.onto(), None);
                // The dropped order is kept: commit 3 becomes the new first commit.
                let order: Vec<String> =
                    state.entries.iter().map(|e| e.commit.to_string()).collect();
                assert_eq!(order, [ids[2].clone(), ids[0].clone(), ids[1].clone()]);
            },
        );
    }
}
