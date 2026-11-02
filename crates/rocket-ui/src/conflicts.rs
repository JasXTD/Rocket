//! Per-tab, bounded text drafts. Background refresh never replaces an editor
//! draft; only an explicit reload or a confirmed save changes its file binding.
//! Binary and delete/modify conflicts are resolved by keeping one whole version.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
    tree::size,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, ops::Range, sync::Arc};

#[cfg(test)]
mod security_tests;

pub enum ConflictIntent {
    Ai(crate::ai_conflict::Action),
    Open(RepoPath),
    /// The conflict editor, then AI resolve once the file is read.
    OpenAi(RepoPath),
    Reload(RepoPath),
    Cancel,
    CancelSave,
    Save(bool),
    Keep(ConflictKeep),
}
#[derive(Default)]
pub struct State {
    pub ai: crate::ai_conflict::State,
    /// AI resolve opens once this read finishes, if the file is a text conflict.
    ai_wanted: bool,
    pub path: Option<RepoPath>,
    pub preview: Option<Arc<ConflictPreview>>,
    pub draft: String,
    pub dirty: bool,
    pub blocks: ConflictBlocks,
    pub selected: usize,
    pub part: u8,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Whole-version review, read with the text when the text editor cannot
    /// represent the conflict: a binary side or file, or a side deleted the path.
    pub whole: Option<Arc<WholeConflictPreview>>,
    pub whole_error: Option<String>,
    /// The version kept by the last confirmed whole-file resolution of `path`.
    pub kept: Option<ConflictKeep>,
    task: Option<ReadTask<Loaded>>,
    jump: Option<usize>,
    pub unlinked: bool,
    pub three_columns: bool,
    /// Ours, Result, Theirs; position and scroll extent, never private text.
    pub scroll_y: [f32; 3],
    pub scroll_max: [f32; 3],
    pub overview_probe: crate::overview::Probe,
    overview: crate::overview::Summary,
    overview_spans: Vec<(usize, usize)>,
    /// Open-item indices in block/document order, rebuilt only when the draft changes.
    overview_open: Vec<usize>,
    overview_jump: Option<f32>,
    shared_row: f32,
    maps: [crate::conflict_scroll::LineMap; 2],
    /// Conflicts resolved here with Use ours, Use theirs or Use both, in draft order.
    resolved: Vec<Resolved>,
    /// The draft at the last change while `resolved` has entries, to see where typing went.
    previous: String,
    /// Every conflict in draft order, open or resolved here, numbered by place from 1.
    items: Vec<(Item, Range<usize>)>,
    /// Where each of `items` sits in the Ours and Theirs files.
    sides: Vec<SideLines>,
    /// The names Git wrote on the markers: ours (such as HEAD), then theirs.
    labels: (String, String),
}
/// A conflict resolved with a choice: where its chosen text sits in the draft, and the
/// original block, markers included, that Undo puts back.
struct Resolved {
    range: Range<usize>,
    original: String,
    /// 2 ours, 3 theirs, 0 both, as `State::choose` takes it, or `AI`.
    choice: u8,
}
/// A `Resolved::choice`: an applied AI proposal, which is neither side as such.
const AI: u8 = 1;
/// One conflict: still open (an index into `blocks`) or resolved here (into `resolved`).
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Open(usize),
    Done(usize),
}
/// The lines one conflict takes in the Ours and Theirs files; none where that side's text
/// is not there as whole lines, such as a side that removed them.
#[derive(Clone, Default)]
struct SideLines {
    ours: Option<Range<usize>>,
    theirs: Option<Range<usize>>,
}
fn shift(range: &Range<usize>, by: isize) -> Range<usize> {
    range.start.saturating_add_signed(by)..range.end.saturating_add_signed(by)
}
/// The lines of `side` that hold `chunk`, the copy nearest line `near`. A side's text in a
/// conflict comes whole from that side's file, so it is there as lines.
fn find_lines(side: &str, chunk: &str, near: usize) -> Option<Range<usize>> {
    if chunk.is_empty() {
        return None;
    }
    let count = chunk.matches('\n').count() + usize::from(!chunk.ends_with('\n'));
    let (mut line, mut counted, mut best) = (0, 0, None::<(usize, usize)>);
    for (at, _) in side.match_indices(chunk) {
        let ends_line = chunk.ends_with('\n') || at + chunk.len() == side.len();
        if (at > 0 && side.as_bytes()[at - 1] != b'\n') || !ends_line {
            continue;
        }
        line += side[counted..at].matches('\n').count();
        counted = at;
        if best.is_none_or(|(distance, _)| line.abs_diff(near) < distance) {
            best = Some((line.abs_diff(near), line));
        }
    }
    best.map(|(_, line)| line..line + count)
}
/// The names Git wrote after the first conflict's markers, such as HEAD and the merged branch.
fn marker_labels(text: &str) -> Option<(String, String)> {
    let block = conflict_blocks(text).blocks.into_iter().next()?;
    let mut lines = text[block.range].lines();
    let label = |line: &str| line.trim_start_matches(['<', '>']).trim().to_owned();
    Some((label(lines.next()?), label(lines.next_back()?)))
}
/// What Ours and Theirs are in the operation under way; they are Git's index sides.
fn roles(state: Option<&OperationState>) -> (&'static str, &'static str) {
    match state {
        Some(s) if s.rebase => ("what your commit goes on top of", "your commit"),
        Some(s) if s.mail_import => ("your branch", "the patch"),
        Some(s) if s.cherry_pick => ("your branch", "picked commit"),
        Some(s) if s.revert => ("your branch", "commit being reverted"),
        _ => ("your branch", "incoming"),
    }
}
/// Ours and Theirs in a sentence, for the conflict strip in Changes.
pub(crate) fn sides(state: Option<&OperationState>) -> String {
    match state {
        Some(s) if s.rebase => "Ours is what your commit goes on top of. Theirs is your commit.".into(),
        Some(s) if s.mail_import => match &s.step {
            Some(step) => format!("Ours is your branch. Theirs is patch {}.", step.at),
            None => "Ours is your branch. Theirs is the patch.".into(),
        },
        Some(s) if s.cherry_pick => "Ours is your branch. Theirs is the commit you picked.".into(),
        Some(s) if s.revert => "Ours is your branch. Theirs is the file before the commit you're reverting.".into(),
        _ => "Ours is your branch. Theirs is the incoming branch.".into(),
    }
}
struct Loaded {
    text: Result<ConflictPreview>,
    whole: Option<Result<WholeConflictPreview>>,
}
impl State {
    pub(crate) fn import_external(&mut self, preview: Arc<ConflictPreview>, text: String) {
        self.forget_resolutions();
        if let Some(labels) = marker_labels(&text) {
            self.labels = labels;
        }
        self.path = Some(preview.path().clone());
        self.preview = Some(preview);
        self.draft = text;
        self.part = 0;
        self.selected = 0;
        self.error = None;
        self.notice = Some(
            "External result imported as a draft. Review and choose Save file or Save and stage."
                .into(),
        );
        self.changed();
        self.jump = Some(0);
    }
    pub fn running(&self) -> bool {
        self.task.is_some() || self.ai.running()
    }
    pub fn stop(&self) {
        self.ai.stop();
        if let Some(task) = &self.task {
            task.cancel();
        }
    }
    fn changed(&mut self) {
        self.follow_edit();
        self.blocks = conflict_blocks(&self.draft);
        self.maps = [2, 3].map(|stage| {
            crate::conflict_scroll::LineMap::new(
                &self.draft,
                self.preview
                    .as_ref()
                    .and_then(|p| p.side(stage))
                    .unwrap_or(""),
            )
        });
        self.selected = self
            .selected
            .min(self.blocks.blocks.len().saturating_sub(1));
        self.dirty = self
            .preview
            .as_ref()
            .is_none_or(|p| self.draft != p.working());
        self.find_sides();
    }
    /// Lists the conflicts in draft order and finds each one's lines in Ours and Theirs.
    fn find_sides(&mut self) {
        let mut items: Vec<_> = self.blocks.blocks.iter().enumerate()
            .map(|(i, b)| (Item::Open(i), b.range.clone()))
            .chain(self.resolved.iter().enumerate().map(|(i, r)| (Item::Done(i), r.range.clone())))
            .collect();
        items.sort_by_key(|(_, range)| (range.start, range.end));
        let side = |stage| self.preview.as_ref().and_then(|p| p.side(stage)).unwrap_or("");
        let (mut line, mut byte) = (0, 0);
        self.sides = items
            .iter()
            .map(|(item, range)| {
                line += self.draft[byte..range.start].bytes().filter(|b| *b == b'\n').count();
                byte = range.start;
                // An open conflict's sides start under its marker line.
                let first = line + usize::from(matches!(item, Item::Open(_)));
                let near = |map: &crate::conflict_scroll::LineMap| map.to_side(first as f32).round() as usize;
                let lines = |text: &str, block: &ConflictBlock| SideLines {
                    ours: find_lines(side(2), &text[block.ours.clone()], near(&self.maps[0])),
                    theirs: find_lines(side(3), &text[block.theirs.clone()], near(&self.maps[1])),
                };
                match *item {
                    Item::Open(i) => self.blocks.blocks.get(i).map(|b| lines(&self.draft, b)),
                    Item::Done(i) => self.resolved.get(i).and_then(|r| {
                        conflict_blocks(&r.original).blocks.first().map(|b| lines(&r.original, b))
                    }),
                }
                .unwrap_or_default()
            })
            .collect();
        self.items = items;
        self.overview_spans = line_spans(&self.draft, self.items.iter().map(|(_, range)| range.clone()));
        self.overview_open = self.items.iter().enumerate().filter_map(|(index, (item, _))| {
            matches!(item, Item::Open(_)).then_some(index)
        }).collect();
        let spans: Vec<_> = self.items.iter().zip(&self.overview_spans).map(|((item, _), (first, last))| {
            (*first..last + 1, if matches!(item, Item::Open(_)) { crate::overview::CONFLICT } else { crate::overview::RESOLVED })
        }).collect();
        self.overview = crate::overview::Summary::from_ranges(self.draft.bytes().filter(|b| *b == b'\n').count() + 1, &spans);
        self.overview_jump = None;
    }
    fn choose(&mut self, which: u8) {
        let Some(block) = self.blocks.blocks.get(self.selected) else {
            return;
        };
        let mut text = String::new();
        if which != 3 {
            text.push_str(&self.draft[block.ours.clone()]);
        }
        if which != 2 {
            text.push_str(&self.draft[block.theirs.clone()]);
        }
        self.resolve(text, which);
    }
    /// Puts `text` in the selected conflict's place. Undo brings the conflict back.
    fn resolve(&mut self, text: String, choice: u8) {
        let Some(block) = self.blocks.blocks.get(self.selected) else {
            return;
        };
        let range = block.range.clone();
        let start = range.start;
        // Undo puts the block back; resolutions after it move with the text.
        let original = self.draft[range.clone()].to_owned();
        let earlier = self.resolved.partition_point(|r| r.range.end <= start);
        let by = text.len() as isize - original.len() as isize;
        for later in &mut self.resolved[earlier..] {
            later.range = shift(&later.range, by);
        }
        self.draft.replace_range(range, &text);
        self.resolved.insert(earlier, Resolved { range: start..start + text.len(), original, choice });
        self.previous.clone_from(&self.draft);
        self.part = 0;
        self.changed();
        self.jump = Some(
            self.blocks
                .blocks
                .get(self.selected)
                .map_or(start, |b| b.range.start),
        );
    }
    /// An AI proposal's text for each open conflict, in order: every one lands resolved,
    /// with its own Undo. Last first, so the earlier conflicts stay where they are.
    pub(crate) fn apply_proposal(&mut self, replacements: Vec<String>) {
        for (index, text) in replacements.into_iter().enumerate().rev() {
            self.selected = index;
            self.resolve(text, AI);
        }
        self.selected = 0;
        self.error = None;
        self.jump = self.resolved.first().map(|r| r.range.start);
        self.notice = Some("AI proposal applied to editor draft. Review and test it, then choose Save file or Save and stage.".into());
    }
    /// Puts a resolved conflict's original block back and selects it.
    fn undo(&mut self, index: usize) {
        if index >= self.resolved.len() {
            return;
        }
        let resolved = self.resolved.remove(index);
        // Every draft change runs through changed(), which keeps ranges current; one that
        // no longer fits is refused rather than allowed to cut the text.
        let Some(chosen) = self.draft.get(resolved.range.clone()) else {
            return;
        };
        let by = resolved.original.len() as isize - chosen.len() as isize;
        let start = resolved.range.start;
        self.draft.replace_range(resolved.range, &resolved.original);
        for later in &mut self.resolved[index..] {
            later.range = shift(&later.range, by);
        }
        self.previous.clone_from(&self.draft);
        self.part = 0;
        self.changed();
        if let Some(block) = self.blocks.blocks.iter().position(|b| b.range.start == start) {
            self.selected = block;
        }
        self.jump = Some(start);
    }
    /// Typing moves the resolutions after the edit and forgets those it touched. The edit
    /// lies between the longest start and the longest end the draft shares with its
    /// previous text; repeated characters let it sit further left or right, and a
    /// resolution that either placement leaves whole is kept while it still starts a
    /// line, since Undo puts back markers that only count at the start of one.
    fn follow_edit(&mut self) {
        if self.resolved.is_empty() {
            self.previous = String::new();
            return;
        }
        if self.previous == self.draft {
            return;
        }
        let (old, new) = (self.previous.as_bytes(), self.draft.as_bytes());
        // Text before `kept` is unchanged in place; text from `moved` on only shifted.
        let kept = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let tail = old.iter().rev().zip(new.iter().rev()).take_while(|(a, b)| a == b).count();
        let moved = old.len() - tail.min(old.len().min(new.len()));
        let by = new.len() as isize - old.len() as isize;
        self.resolved.retain_mut(|r| {
            // Wholly before `kept` it stays put; wholly from `moved` on it shifts; else touched.
            if r.range.end > kept {
                if r.range.start < moved {
                    return false;
                }
                r.range = shift(&r.range, by);
            }
            r.range.start == 0 || new.get(r.range.start - 1) == Some(&b'\n')
        });
        if self.resolved.is_empty() {
            self.previous = String::new();
        } else {
            self.previous.clone_from(&self.draft);
        }
    }
    /// A new text for the editor: earlier choices can no longer be undone.
    fn forget_resolutions(&mut self) {
        self.resolved.clear();
        self.previous = String::new();
        self.overview = Default::default();
        self.overview_spans.clear();
        self.overview_open.clear();
        self.overview_probe = Default::default();
        self.overview_jump = None;
    }
}
impl Rocket {
    pub(crate) fn conflict_intent(&mut self, intent: ConflictIntent) {
        if let ConflictIntent::Ai(action)=intent { self.ai_conflict_intent(action); return; }
        if matches!(intent, ConflictIntent::CancelSave) {
            self.cancel_write();
            return;
        }
        if self.write.is_some() || self.quitting {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        match intent {
            ConflictIntent::Ai(_) => unreachable!(),
            ConflictIntent::CancelSave => unreachable!(),
            ConflictIntent::Cancel => tab.conflict.stop(),
            ConflictIntent::OpenAi(path) => {
                let same = tab.conflict.path.as_ref() == Some(&path);
                // Unsaved text of another file asks first; AI resolve can wait for a click.
                tab.conflict.ai_wanted = same || !tab.conflict.dirty;
                self.conflict_intent(ConflictIntent::Open(path));
            }
            ConflictIntent::Open(path) => {
                tab.view = View::Conflict;
                if tab.conflict.path.as_ref() == Some(&path)
                    && (tab.conflict.preview.is_some()
                        || tab.conflict.whole.is_some()
                        || tab.conflict.dirty)
                {
                    return;
                }
                if tab.conflict.dirty {
                    self.show_dialog(DialogKind::ConflictReload(path));
                } else {
                    let id = tab.id;
                    self.load_conflict(id, path);
                }
            }
            ConflictIntent::Reload(path) => self.show_dialog(DialogKind::ConflictReload(path)),
            ConflictIntent::Save(stage) => {
                let state = &tab.conflict;
                if state.running() {
                    return;
                }
                let Some(preview) = state.preview.clone() else {
                    return;
                };
                if stage && !state.blocks.is_resolved() {
                    return;
                }
                let text = state.draft.clone();
                let label = if stage { "Save and stage" } else { "Save file" };
                let message = format!(
                    "{label}: {}\n\nReplace the working file with this result ({} bytes of editor text). {}",
                    preview.path().display_lossy(),
                    text.len(),
                    if stage {
                        "Stage only this path. This does not commit or continue the surrounding operation."
                    } else {
                        "The index is left unchanged; unfinished conflict markers may remain."
                    }
                );
                self.show_dialog(DialogKind::Confirm(
                    Action::SaveConflict {
                        preview,
                        text,
                        stage,
                    },
                    message,
                ));
            }
            ConflictIntent::Keep(keep) => {
                let state = &tab.conflict;
                let Some(preview) = state.whole.clone().filter(|p| p.can_keep(keep)) else {
                    return;
                };
                if state.running() {
                    return;
                }
                let message = keep_message(&preview, keep, state.dirty);
                self.show_dialog(DialogKind::Confirm(
                    Action::KeepConflictSide { preview, keep },
                    message,
                ));
            }
        }
    }
    pub(crate) fn load_conflict(&mut self, owner: u64, path: RepoPath) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == owner) else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        tab.view = View::Conflict;
        tab.diff = None;
        let state = &mut tab.conflict;
        state.ai.reset();
        state.task = None;
        state.error = None;
        state.whole_error = None;
        state.notice = Some("Reading file and conflict stages…".into());
        if state.path.is_none() {
            state.path = Some(path.clone());
        }
        // Keep the previous draft if reading fails or is cancelled. The receiver
        // is replaced on every request, so stale completions cannot reach the UI.
        match ReadTask::spawn(move |cancel| {
            let text = backend.log.request("conflict-edit", |log| {
                repo.with_log(log.clone()).edit_conflict(&path, &cancel)
            });
            cancel.check()?;
            // Review whole versions only when the text editor cannot show the
            // conflict: a binary or unreadable file, or a side that deleted it.
            let whole = match &text {
                Ok(preview) => preview.side(2).is_none() || preview.side(3).is_none(),
                Err(error) => matches!(error, Error::Conflict(ConflictFailure::UnsupportedFile)),
            }
            .then(|| {
                backend.log.request("conflict-keep-review", |log| {
                    repo.with_log(log.clone())
                        .review_whole_conflict(&path, &cancel)
                })
            });
            cancel.check()?;
            Ok(Loaded { text, whole })
        }) {
            Ok(task) => state.task = Some(task),
            Err(error) => state.error = Some(user_error(&error)),
        }
    }
    pub(crate) fn poll_conflicts(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            let s = &mut tab.conflict;
            crate::ai_conflict::poll(s,ctx);
            if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
                s.task = None;
                s.notice = None;
                match result {
                    Ok(Loaded { text, whole }) => {
                        s.kept = None;
                        (s.whole, s.whole_error) = match whole {
                            Some(Ok(review)) => (Some(Arc::new(review)), None),
                            Some(Err(error)) => (None, Some(user_error(&error))),
                            None => (None, None),
                        };
                        match text {
                            Ok(preview) => {
                                s.forget_resolutions();
                                s.path = Some(preview.path().clone());
                                s.draft = preview.working().into();
                                s.labels = marker_labels(&s.draft).unwrap_or_default();
                                s.preview = Some(Arc::new(preview));
                                s.error = None;
                                s.part = 0;
                                s.selected = 0;
                                s.jump = Some(0);
                                s.changed();
                            }
                            Err(error) => {
                                // A whole-version review is a successful read of this
                                // path: bind to it rather than keep an earlier file's
                                // text. Opening another path with unsaved text asks first.
                                if let Some(review) = &s.whole {
                                    s.path = Some(review.path().clone());
                                    s.preview = None;
                                    s.forget_resolutions();
                                    s.draft.clear();
                                    s.dirty = false;
                                    s.blocks = ConflictBlocks::default();
                                    s.selected = 0;
                                    s.part = 0;
                                }
                                // LFS/gitlink paths intentionally bypass text editing.
                                // Their successful whole-version preview is the UI,
                                // not an unsupported-text error above usable choices.
                                s.error = (!s.whole.as_ref().is_some_and(|review| {
                                    review.kind() != ConflictKind::File
                                })).then(|| user_error(&error));
                            }
                        }
                    }
                    Err(Error::Cancelled) => {
                        s.notice =
                            Some("Read cancelled. Existing draft retained; reload to retry.".into())
                    }
                    Err(error) => s.error = Some(user_error(&error)),
                }
            }
            if s.running() {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
    }
    pub(crate) fn finish_conflict_write(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) else {
            return;
        };
        let s = &mut tab.conflict;
        s.notice = Some(outcome.summary.clone());
        if outcome.command_succeeded {
            s.error = None;
            if let Some(report) = &outcome.conflict {
                s.preview = report.preview.clone();
                s.dirty = false;
            }
        } else {
            s.error = Some(
                outcome
                    .details
                    .first()
                    .cloned()
                    .unwrap_or_else(|| outcome.summary.clone()),
            );
            // Require a reviewed reload after an uncertain or refused write.
            // The user's text remains available to copy, even after staging fails.
            s.preview = None;
            s.dirty = true;
            s.forget_resolutions();
        }
    }
    pub(crate) fn conflict_save_cancelled(&mut self, owner: Option<u64>) {
        if let Some(backend) = &self.backend {
            let _: rocket_git::Result<()> = backend
                .log
                .request("conflict-save-cancel", |_| Err(Error::Cancelled));
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) {
            tab.conflict.notice =
                Some("Save cancelled. Draft retained; no file or index write was started.".into());
        }
    }
    pub(crate) fn finish_conflict_keep(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) else {
            return;
        };
        let s = &mut tab.conflict;
        s.notice = Some(outcome.summary.clone());
        // A completed, refused or uncertain write ends this review; only an
        // explicit reload may bind the editor to the path again.
        s.whole = None;
        let changed = outcome
            .whole_conflict
            .as_ref()
            .is_some_and(|c| c.file_change.is_some());
        if outcome.command_succeeded {
            s.error = None;
            s.whole_error = None;
            s.kept = outcome.whole_conflict.as_ref().map(|c| c.keep);
            // The path is resolved, so its text binding and draft no longer apply.
            s.preview = None;
            s.forget_resolutions();
            s.draft.clear();
            s.dirty = false;
            s.blocks = ConflictBlocks::default();
            s.selected = 0;
            s.part = 0;
        } else {
            s.error = Some(
                outcome
                    .details
                    .first()
                    .cloned()
                    .unwrap_or_else(|| outcome.summary.clone()),
            );
            // The working file may differ from any text binding; the draft stays
            // available to copy.
            if changed {
                s.preview = None;
            }
        }
    }
    pub(crate) fn conflict_keep_cancelled(&mut self, owner: Option<u64>) {
        if let Some(backend) = &self.backend {
            let _: rocket_git::Result<()> = backend
                .log
                .request("conflict-keep-cancel", |_| Err(Error::Cancelled));
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner) {
            tab.conflict.notice =
                Some("Resolution cancelled; no file or index write was started.".into());
        }
    }
}
fn register(
    ui: &Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    rect: Rect,
    enabled: bool,
) {
    let rect = rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(key.into(), UiControl { rect, enabled });
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    register(ui, widgets, key, r.interact_rect, r.enabled());
    r.clicked()
}
/// Room for line numbers in the result editor's left margin.
const GUTTER: i8 = 56;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Part {
    Common,
    Marker,
    Ours,
    Base,
    Theirs,
}

/// Each line's place in a conflict: a marker, either side, the diff3 base, or outside.
pub(crate) fn line_parts(text: &str) -> Vec<Part> {
    let mut part = Part::Common;
    text.split('\n')
        .map(|line| {
            let marker = |m: &str| {
                line.starts_with(m) && matches!(line.as_bytes().get(7), None | Some(b' ' | b'\r'))
            };
            if marker("<<<<<<<") {
                part = Part::Ours;
                Part::Marker
            } else if part != Part::Common && marker("|||||||") {
                part = Part::Base;
                Part::Marker
            } else if part != Part::Common && marker("=======") {
                part = Part::Theirs;
                Part::Marker
            } else if part != Part::Common && marker(">>>>>>>") {
                part = Part::Common;
                Part::Marker
            } else {
                part
            }
        })
        .collect()
}

/// The result text for the editor: markers and the base dimmed, one section per run.
fn conflict_job(text: &str, font: &FontId, wrap_width: f32) -> text::LayoutJob {
    let parts = line_parts(text);
    let color = |part: Part| match part {
        Part::Marker => HINT(),
        Part::Base => FAINT(),
        _ => TEXT(),
    };
    let mut job = text::LayoutJob::default();
    let (mut start, mut at, mut run) = (0, 0, None);
    for (line, part) in text.split_inclusive('\n').zip(&parts) {
        let ink = color(*part);
        if run != Some(ink) {
            if let Some(ink) = run {
                job.append(&text[start..at], 0.0, TextFormat::simple(font.clone(), ink));
            }
            (start, run) = (at, Some(ink));
        }
        at += line.len();
    }
    job.append(&text[start..at], 0.0, TextFormat::simple(font.clone(), run.unwrap_or(TEXT())));
    job.wrap.max_width = wrap_width;
    job
}

/// `text` laid out for its editor by `job`, given a text and a width. Each of `chip_lines`, in
/// order, has a chip `room` wide at its end and wraps before it; other lines keep the whole width.
fn chip_galley(
    ui: &Ui,
    text: &str,
    wrap_width: f32,
    chip_lines: &[usize],
    room: f32,
    job: impl Fn(&str, f32) -> text::LayoutJob,
) -> Arc<Galley> {
    if chip_lines.is_empty() || text.is_empty() {
        return ui.fonts_mut(|f| f.layout_job(job(text, wrap_width)));
    }
    // Neighbouring lines of one width are laid out together.
    let (mut runs, mut at) = (Vec::<(Range<usize>, bool)>::new(), 0);
    for (line, body) in text.split_inclusive('\n').enumerate() {
        let chip = chip_lines.binary_search(&line).is_ok();
        match runs.last_mut() {
            Some((run, same)) if *same == chip => run.end += body.len(),
            _ => runs.push((at..at + body.len(), chip)),
        }
        at += body.len();
    }
    let galleys: Vec<_> = runs.iter().enumerate().map(|(k, (run, chip))| {
        // Galley::concat puts back the newline between two runs, as egui's own paragraphs do.
        let body = &text[run.clone()];
        let body = if k + 1 < runs.len() { body.strip_suffix('\n').unwrap_or(body) } else { body };
        ui.fonts_mut(|f| f.layout_job(job(body, if *chip { wrap_width - room } else { wrap_width })))
    }).collect();
    Arc::new(Galley::concat(Arc::new(job(text, wrap_width)), &galleys, ui.ctx().pixels_per_point()))
}

/// Each range's first and last line, for ranges in draft order.
fn line_spans(text: &str, ranges: impl Iterator<Item = Range<usize>>) -> Vec<(usize, usize)> {
    let (mut line, mut byte) = (0, 0);
    ranges
        .map(|range| {
            line += text[byte..range.start].bytes().filter(|b| *b == b'\n').count();
            byte = range.start;
            // Its last line is the one its final newline ends.
            let body = &text[range];
            let body = body.strip_suffix('\n').unwrap_or(body);
            (line, line + body.bytes().filter(|b| *b == b'\n').count())
        })
        .collect()
}

/// Each logical line's top and bottom on screen, and the width of its first row.
fn line_rows(output: &text_edit::TextEditOutput) -> Vec<(f32, f32, f32)> {
    let origin = output.galley_pos;
    let mut lines: Vec<(f32, f32, f32)> = Vec::new();
    let mut starts = true;
    for row in &output.galley.rows {
        let r = row.rect().translate(origin.to_vec2());
        if starts {
            lines.push((r.top(), r.bottom(), row.rect().width()));
        } else if let Some(last) = lines.last_mut() {
            last.1 = r.bottom();
        }
        starts = row.ends_with_newline;
    }
    lines
}

/// The middle of the row of `line` nearest `middle` whose text stops short of `x`, for words
/// starting there. A wrapped line's rows differ, so each is measured.
fn row_with_room(output: &text_edit::TextEditOutput, line: usize, x: f32, middle: f32) -> Option<f32> {
    output.galley.rows.split_inclusive(|row| row.ends_with_newline).nth(line).unwrap_or_default().iter()
        .map(|row| row.rect().translate(output.galley_pos.to_vec2()))
        .filter(|row| row.right() + 12.0 <= x)
        .map(|row| row.center().y)
        .min_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()))
}

/// Line numbers right-aligned in the gutter, for the lines on screen.
fn paint_numbers(painter: &Painter, lines: &[(f32, f32, f32)], text_left: f32, clip: Rect) {
    let font = FontId::monospace(11.5);
    for (line, (top, bottom, _)) in lines.iter().enumerate() {
        if *bottom >= clip.top() && *top <= clip.bottom() {
            painter.text(pos2(text_left - 12.0, (top + bottom) / 2.0), Align2::RIGHT_CENTER, (line + 1).to_string(), font.clone(), FAINT());
        }
    }
}

/// A conflict's number at the left of the gutter: amber while open, green once resolved.
pub(crate) fn badge(painter: &Painter, center: Pos2, number: usize, open: bool) {
    let (fill, ink) = if open { (AMBER(), AMBER_INK()) } else { (GREEN(), GREEN_INK()) };
    painter.circle_filled(center, 7.0, fill);
    painter.text(center, Align2::CENTER_CENTER, number.to_string(), semibold(10.5), ink);
}

/// Room each side of a chip's label.
const CHIP_PAD: f32 = 6.0;

fn chip_width(ui: &Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(11.0), TEXT()).size().x + 2.0 * CHIP_PAD + 4.0
}

/// A button no taller than the text row it sits on, so it never covers the next line.
fn chip(ui: &mut Ui, at: Rect, label: &str, enabled: bool) -> Response {
    ui.put(at, |ui: &mut Ui| {
        ui.spacing_mut().button_padding = vec2(CHIP_PAD, 0.0);
        // A button is never shorter than interact_size, the theme's control height.
        ui.spacing_mut().interact_size.y = at.height();
        ui.add_enabled(enabled, Button::new(RichText::new(label).size(11.0)).wrap_mode(TextWrapMode::Extend).min_size(at.size()))
    })
}

/// What a chip in a pane asked for.
enum Pick {
    /// A conflict and side (2 ours, 3 theirs, 0 both), as `State::choose` takes it.
    Choose(usize, u8),
    /// A resolved conflict to put back, as `State::undo` takes it.
    Undo(usize),
}

/// Each conflict's rows on screen in one pane: its place in `State::items`, top, bottom.
type Rows = Vec<(usize, f32, f32)>;

/// Paints Result's tinted sides, numbers and the selected conflict's bar, and puts the
/// choices on each conflict's first line: all three in one column, only Use both beside
/// the side panes unless a side's lines were not found there. A conflict resolved here is
/// tinted green, with its name and Undo on its first line.
fn decorate(
    ui: &mut Ui,
    s: &State,
    output: &text_edit::TextEditOutput,
    backdrop: layers::ShapeIdx,
    editable: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> (Option<Pick>, Rows) {
    let ready = editable && !s.blocks.malformed;
    let rect = output.response.response.rect;
    let origin = output.galley_pos;
    let clip = ui.clip_rect();
    let parts = line_parts(&s.draft);
    let lines = line_rows(output);
    let mut shapes = Vec::new();
    let left = rect.left() + f32::from(GUTTER) - 6.0;
    for (line, (top, bottom, _)) in lines.iter().enumerate() {
        if *bottom < clip.top() || *top > clip.bottom() {
            continue;
        }
        let tint = match parts.get(line).copied().unwrap_or(Part::Common) {
            Part::Ours => BLUE().gamma_multiply(0.10),
            Part::Theirs => VIOLET().gamma_multiply(0.12),
            Part::Marker | Part::Base => MUTED().gamma_multiply(0.05),
            Part::Common => continue,
        };
        shapes.push(Shape::rect_filled(Rect::from_min_max(pos2(left, *top), pos2(rect.right(), *bottom)), 0.0, tint));
    }
    let spans = line_spans(&s.draft, s.items.iter().map(|(_, range)| range.clone()));
    let bar = |shapes: &mut Vec<Shape>, top: f32, bottom: f32, color| {
        shapes.push(Shape::rect_filled(Rect::from_min_max(pos2(left, top), pos2(left + 3.0, bottom)), 1.0, color));
    };
    let mut rows = Rows::new();
    for (k, ((item, range), (first, last))) in s.items.iter().zip(&spans).enumerate() {
        let (Some(top), Some(bottom)) = (lines.get(*first), lines.get(*last)) else { continue };
        rows.push((k, top.0, bottom.1));
        match *item {
            Item::Done(_) if !range.is_empty() => {
                shapes.push(Shape::rect_filled(Rect::from_min_max(pos2(left, top.0), pos2(rect.right(), bottom.1)), 0.0, GREEN().gamma_multiply(0.07)));
                bar(&mut shapes, top.0, bottom.1, GREEN());
            }
            Item::Open(block) if block == s.selected => bar(&mut shapes, top.0, bottom.1, AMBER()),
            _ => (),
        }
    }
    ui.painter().set(backdrop, Shape::Vec(shapes));
    let painter = ui.painter().with_clip_rect(clip);
    paint_numbers(&painter, &lines, origin.x, clip);
    let mut chosen = None;
    let full = [("conflict-ours", "Use ours", 2), ("conflict-theirs", "Use theirs", 3), ("conflict-both", "Use both", 0)];
    let short = ["Ours", "Theirs", "Both"];
    for (k, ((item, _), (first, _))) in s.items.iter().zip(&spans).enumerate() {
        let Some((top, bottom, line_width)) = lines.get(*first).copied() else { continue };
        if bottom < clip.top() || top > clip.bottom() {
            continue;
        }
        let middle = (top + bottom) / 2.0;
        badge(&painter, pos2(rect.left() + 10.0, middle), k + 1, matches!(item, Item::Open(_)));
        let height = (bottom - top).max(12.0);
        match *item {
            Item::Open(block) => {
                // Beside the side panes, each side's choice sits on that side's lines.
                let side = s.sides.get(k).cloned().unwrap_or_default();
                let shown: Vec<usize> = (0..3)
                    .filter(|c| match c {
                        0 => !s.three_columns || side.ours.is_none(),
                        1 => !s.three_columns || side.theirs.is_none(),
                        _ => true,
                    })
                    .collect();
                let width = |labels: &[&str]| -> f32 {
                    labels.iter().map(|l| chip_width(ui, l)).sum::<f32>() + 4.0 * labels.len().saturating_sub(1) as f32
                };
                let long: Vec<&str> = shown.iter().map(|&c| full[c].1).collect();
                let brief: Vec<&str> = shown.iter().map(|&c| short[c]).collect();
                // Beside the marker text when there is room, else shorter words.
                let room = rect.right() - 8.0 - (origin.x + line_width + 12.0);
                let labels = if width(&long) <= room { long } else { brief };
                let mut right = rect.right() - 8.0;
                for (&c, label) in shown.iter().zip(labels).rev() {
                    let (key, _, which) = full[c];
                    let w = chip_width(ui, label);
                    let at = Rect::from_min_size(pos2(right - w, middle - height / 2.0), vec2(w, height));
                    right -= w + 4.0;
                    let r = chip(ui, at, label, ready);
                    register(ui, widgets, &format!("{key}-{block}"), r.interact_rect, ready);
                    // The selected conflict's choices keep the toolbar's old names.
                    if block == s.selected {
                        register(ui, widgets, key, r.interact_rect, ready);
                    }
                    if ready && r.clicked() {
                        chosen = Some(Pick::Choose(block, which));
                    }
                }
            }
            Item::Done(index) => {
                // Undo at the end, and the conflict's name on a row of the line that leaves room.
                let Some(resolved) = s.resolved.get(index) else { continue };
                let w = chip_width(ui, "Undo");
                let at = Rect::from_min_size(pos2(rect.right() - 8.0 - w, middle - height / 2.0), vec2(w, height));
                let undo = chip(ui, at, "Undo", editable).on_hover_text(format!("Put conflict {} back", k + 1));
                register(ui, widgets, &format!("conflict-undo-{index}"), undo.interact_rect, editable);
                if editable && undo.clicked() {
                    chosen = Some(Pick::Undo(index));
                }
                let used = match resolved.choice {
                    AI => " · AI",
                    2 => " · resolved · used ours",
                    3 => " · resolved · used theirs",
                    _ => " · resolved · used both",
                };
                let brief = if resolved.choice == AI { used } else { " · resolved" };
                for detail in [used, brief, ""] {
                    let mut job = text::LayoutJob::default();
                    job.append(&format!("Conflict {}", k + 1), 0.0, TextFormat::simple(semibold(11.5), TEXT()));
                    job.append(detail, 0.0, TextFormat::simple(FontId::proportional(11.5), MUTED()));
                    let galley = ui.painter().layout_job(job);
                    // A check mark, then the words.
                    let x = at.left() - 6.0 - (16.0 + galley.size().x);
                    if let Some(y) = row_with_room(output, *first, x, middle) {
                        icon(&painter, Rect::from_center_size(pos2(x + 6.0, y), vec2(11.0, 11.0)), Icon::Check, GREEN());
                        painter.galley(pos2(x + 16.0, y - galley.size().y / 2.0), galley, TEXT());
                        break;
                    }
                }
            }
        }
    }
    (chosen, rows)
}

/// A side pane's choice, pointing at Result when it stands beside it.
fn side_choice(index: usize, beside: bool) -> &'static str {
    match (index, beside) {
        (0, true) => "Use ours →",
        (0, false) => "Use ours",
        (_, true) => "← Use theirs",
        (_, false) => "Use theirs",
    }
}

/// Paints a side pane's numbers and, for each conflict found there, its lines in the
/// side's color with the conflict's number: Use ours or Use theirs on the first line while
/// it is open, then whether the resolution used this side.
fn decorate_side(
    ui: &mut Ui,
    s: &State,
    output: &text_edit::TextEditOutput,
    backdrop: layers::ShapeIdx,
    index: usize,
    editable: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> (Option<Pick>, Rows) {
    let ready = editable && !s.blocks.malformed;
    let rect = output.response.response.rect;
    let clip = ui.clip_rect();
    let lines = line_rows(output);
    let (tint, which, key) = if index == 0 { (BLUE(), 2, "conflict-ours") } else { (VIOLET(), 3, "conflict-theirs") };
    let label = side_choice(index, s.three_columns);
    let left = rect.left() + f32::from(GUTTER) - 6.0;
    let mut shapes = Vec::new();
    let mut rows = Rows::new();
    let mut marks = Vec::new();
    for (k, ((item, _), side)) in s.items.iter().zip(&s.sides).enumerate() {
        let Some(range) = (if index == 0 { &side.ours } else { &side.theirs }) else { continue };
        let (Some(first), Some(last)) = (lines.get(range.start), range.end.checked_sub(1).and_then(|l| lines.get(l))) else {
            continue;
        };
        // Once resolved: whether that choice took this side's lines.
        let used = match *item {
            Item::Done(r) => s.resolved.get(r).filter(|r| r.choice != AI).map(|r| r.choice == which || r.choice == 0),
            Item::Open(_) => None,
        };
        let faded = used == Some(false);
        shapes.push(Shape::rect_filled(
            Rect::from_min_max(pos2(left, first.0), pos2(rect.right(), last.1)),
            0.0,
            tint.gamma_multiply(if faded { 0.05 } else { 0.12 }),
        ));
        shapes.push(Shape::rect_filled(
            Rect::from_min_max(pos2(left, first.0), pos2(left + 3.0, last.1)),
            1.0,
            if faded { tint.gamma_multiply(0.45) } else { tint },
        ));
        rows.push((k, first.0, last.1));
        marks.push((k, range.start, *first, *item, used));
    }
    ui.painter().set(backdrop, Shape::Vec(shapes));
    let painter = ui.painter().with_clip_rect(clip);
    paint_numbers(&painter, &lines, output.galley_pos.x, clip);
    let mut chosen = None;
    for (k, line, (top, bottom, _), item, used) in marks {
        if bottom < clip.top() || top > clip.bottom() {
            continue;
        }
        let middle = (top + bottom) / 2.0;
        badge(&painter, pos2(rect.left() + 10.0, middle), k + 1, matches!(item, Item::Open(_)));
        let height = (bottom - top).max(12.0);
        match (item, used) {
            (Item::Open(block), _) => {
                let w = chip_width(ui, label);
                let at = Rect::from_min_size(pos2(rect.right() - 8.0 - w, middle - height / 2.0), vec2(w, height));
                let r = chip(ui, at, label, ready);
                register(ui, widgets, &format!("{key}-{block}"), r.interact_rect, ready);
                // The selected conflict's choice keeps its old name.
                if block == s.selected {
                    register(ui, widgets, key, r.interact_rect, ready);
                }
                if ready && r.clicked() {
                    chosen = Some(Pick::Choose(block, which));
                }
            }
            (Item::Done(_), Some(used)) => {
                let (text, color) = if used { ("Used", GREEN()) } else { ("Not used", MUTED()) };
                let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color);
                let x = rect.right() - 8.0 - galley.size().x;
                // Only where the line leaves room; the badge's color already tells.
                if let Some(y) = row_with_room(output, line, x - 16.0, middle) {
                    if used {
                        icon(&painter, Rect::from_center_size(pos2(x - 9.0, y), vec2(11.0, 11.0)), Icon::Check, GREEN());
                    }
                    painter.galley(pos2(x, y - galley.size().y / 2.0), galley, color);
                }
            }
            (Item::Done(_), None) => (),
        }
    }
    (chosen, rows)
}

/// A pane's name strip: a swatch in its color, its name, what it holds, and whether it edits.
pub(crate) fn pane_header(ui: &mut Ui, title: &str, detail: &str, access: &str, tint: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, PANEL());
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let swatch = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(10.0, 10.0));
    painter.rect_filled(swatch, 3.0, tint);
    let name = painter.layout_no_wrap(title.to_owned(), semibold(12.5), TEXT());
    let access = painter.layout_no_wrap(access.to_owned(), FontId::proportional(12.0), FAINT());
    let x = swatch.right() + 8.0;
    let end = rect.right() - 10.0 - access.size().x;
    painter.galley(pos2(end, rect.center().y - access.size().y / 2.0), access, FAINT());
    let name_width = name.size().x;
    painter.galley(pos2(x, rect.center().y - name.size().y / 2.0), name, TEXT());
    let room = Rect::from_min_max(pos2(x + name_width + 8.0, rect.top()), pos2(end - 8.0, rect.bottom()));
    if room.width() > 0.0 {
        clipped_text(ui, room, detail, 12.0, MUTED());
    }
}

/// What one pane reported this frame: a scroll the user made, its box and viewport on
/// screen, and each conflict's rows there, for the bands between panes.
struct PaneOut {
    moved: Option<f32>,
    frame: Rect,
    view: Rect,
    rows: Rows,
}
impl Default for PaneOut {
    fn default() -> Self {
        Self { moved: None, frame: Rect::NOTHING, view: Rect::NOTHING, rows: Rows::new() }
    }
}

/// One full-height pane: a box with a name strip over its text, Ours and Theirs read only
/// and Result editable, all with the same monospace line height. Common-line anchors link
/// vertical positions, not percentages.
#[allow(clippy::too_many_arguments)]
fn column(
    ui: &mut Ui,
    s: &mut State,
    id: u64,
    index: usize,
    editable: bool,
    force_scroll: bool,
    names: &[String; 2],
    widgets: &mut HashMap<String, UiControl>,
) -> PaneOut {
    let (title, detail, access, tint, key) = match index {
        0 => ("Ours", names[0].as_str(), "Read only", BLUE(), "conflict-ours-scroll"),
        1 => ("Result", "Saved to the file", "Editable", GREEN(), "conflict-editor-scroll"),
        _ => ("Theirs", names[1].as_str(), "Read only", VIOLET(), "conflict-theirs-scroll"),
    };
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let row = match index {
        0 => s.maps[0].to_side(s.shared_row),
        2 => s.maps[1].to_side(s.shared_row),
        _ => s.shared_row,
    };
    let expected = row * row_height;
    let controlled = !s.unlinked || force_scroll;
    let mut out = PaneOut::default();
    let mut actual = (0.0, 0.0);
    let frame = Frame::new().fill(DEEP()).stroke(Stroke::new(1.0, CARD_LINE())).corner_radius(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        pane_header(ui, title, detail, access, tint);
        let height = ui.available_height().max(24.0);
        let mut scroll = ScrollArea::both()
            .id_salt(("conflict-column", id, index))
            .max_height(height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false]);
        if controlled {
            // egui clamps an offset only while a scroll bar shows, so a pane that fits
            // would keep a jump's offset and show empty space; its last extent bounds it.
            scroll = scroll.vertical_scroll_offset(expected.min(s.scroll_max[index]));
        }
        if index == 1 && let Some(offset) = s.overview_jump.take() {
            scroll = scroll.vertical_scroll_offset(offset.min(s.scroll_max[index]));
        }
        let width = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let body = ui.allocate_ui(vec2((width - if index == 1 { crate::overview::WIDTH + 2.0 } else { 0.0 }).max(40.0), height), |ui| {
                let o = scroll.show(ui, |ui| {
                    // The tints and bars go behind the text.
                    let backdrop = ui.painter().add(Shape::Noop);
                    // With a custom frame egui insets the text by the frame, not margin().
                    let inset = Frame::new().inner_margin(Margin { left: GUTTER, right: 8, top: 2, bottom: 2 });
                    let (pick, rows) = if index == 1 {
                        let font = TextStyle::Monospace.resolve(ui.style());
                        let undo_lines: Vec<usize> = s.items.iter().zip(&s.overview_spans)
                            .filter(|((item, _), _)| matches!(item, Item::Done(_)))
                            .map(|(_, (first, _))| *first)
                            .collect();
                        let room = chip_width(ui, "Undo") + 6.0;
                        let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
                            chip_galley(ui, buffer.as_str(), wrap_width, &undo_lines, room, |text, width| conflict_job(text, &font, width))
                        };
                        let output = ui
                            .add_enabled_ui(editable, |ui| {
                                TextEdit::multiline(&mut s.draft)
                                    .id(Id::new(("conflict-result", id)))
                                    .code_editor()
                                    .frame(inset)
                                    .desired_width(ui.available_width().max(40.0))
                                    .desired_rows(1)
                                    .char_limit(CONFLICT_TEXT_LIMIT)
                                    .layouter(&mut layouter)
                                    .show(ui)
                            })
                            .inner;
                        let r = &output.response.response;
                        register(ui, widgets, "conflict-result", r.interact_rect, r.enabled());
                        if r.changed() {
                            s.changed();
                        }
                        decorate(ui, s, &output, backdrop, editable, widgets)
                    } else {
                        // A read-only editor: its text can be selected and copied.
                        let preview = s.preview.clone();
                        let mut text: &str = preview
                            .as_deref()
                            .and_then(|p| p.side(if index == 0 { 2 } else { 3 }))
                            .unwrap_or("This side has no file.");
                        let font = TextStyle::Monospace.resolve(ui.style());
                        let ink = ui.visuals().override_text_color.unwrap_or_else(|| ui.visuals().widgets.inactive.text_color());
                        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
                        let mut choice_lines: Vec<usize> = s.items.iter().zip(&s.sides)
                            .filter(|((item, _), _)| matches!(item, Item::Open(_)))
                            .filter_map(|(_, side)| if index == 0 { side.ours.as_ref() } else { side.theirs.as_ref() })
                            .map(|lines| lines.start)
                            .collect();
                        choice_lines.sort_unstable();
                        let room = chip_width(ui, side_choice(index, s.three_columns)) + 6.0;
                        let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
                            chip_galley(ui, buffer.as_str(), wrap_width, &choice_lines, room, |text, width| {
                                // As TextEdit lays out plain text by itself.
                                let mut job = text::LayoutJob::simple(text.to_owned(), font.clone(), ink, width);
                                job.keep_trailing_whitespace = true;
                                for section in &mut job.sections {
                                    section.format.line_height = Some(line_height);
                                }
                                job
                            })
                        };
                        let output = TextEdit::multiline(&mut text)
                            .id(Id::new(("conflict-side", id, index)))
                            .code_editor()
                            .frame(inset)
                            .desired_width(ui.available_width().max(40.0))
                            .desired_rows(1)
                            .layouter(&mut layouter)
                            .show(ui);
                        decorate_side(ui, s, &output, backdrop, index, editable, widgets)
                    };
                    out.rows = rows;
                    match pick {
                        Some(Pick::Choose(block, which)) => {
                            s.selected = block;
                            s.choose(which);
                        }
                        Some(Pick::Undo(resolved)) => s.undo(resolved),
                        None => (),
                    }
                });
                register(ui, widgets, key, o.inner_rect, true);
                out.view = o.inner_rect;
                actual = (o.state.offset.y, (o.content_size.y - o.inner_rect.height()).max(0.0));
            });
            if index == 1 {
                rail(ui, s, id, body.response.rect.height(), row_height,
                    crate::overview::View { offset: actual.0, extent: actual.1 + out.view.height(), viewport: out.view.height() }, widgets);
            }
        });
    });
    out.frame = frame.response.rect;
    s.scroll_y[index] = actual.0;
    s.scroll_max[index] = actual.1;
    // Ignore bottom clamping, which must not pull the longer panes upwards.
    if controlled && !force_scroll && (actual.0 - expected.min(actual.1)).abs() > 0.75 {
        let row = actual.0 / row_height;
        out.moved = Some(match index {
            0 => s.maps[0].to_result(row),
            2 => s.maps[1].to_result(row),
            _ => row,
        });
    }
    out
}

const RAIL_CONTROL_LIMIT: usize = 256;
const RAIL_PAINT_LIMIT: usize = 1024;

fn rail_mark(s: &State, rail: Rect, per_line: f32, index: usize) -> Rect {
    let (first, last) = s.overview_spans[index];
    let top = rail.top() + 2.0 + first as f32 * per_line;
    let size = vec2(rail.width() - 2.0, ((last - first + 1) as f32 * per_line).max(7.0));
    Rect::from_min_size(pos2(rail.left() + 1.0, top), size).intersect(rail.shrink(1.0))
}

/// The last overlapping open marker wins, as before, without scanning every
/// conflict. The cached indices and marker tops are both in document order.
fn rail_hit(s: &State, rail: Rect, per_line: f32, pointer: Pos2) -> Option<(usize, usize)> {
    if !rail.shrink(1.0).contains(pointer) { return None; }
    let end = s.overview_open.partition_point(|&index| {
        rail.top() + 2.0 + s.overview_spans[index].0 as f32 * per_line <= pointer.y
    });
    let index = *s.overview_open.get(end.checked_sub(1)?)?;
    let (Item::Open(block), range) = &s.items[index] else { return None };
    rail_mark(s, rail, per_line, index).contains(pointer).then_some((*block, range.start))
}

/// Bounded overview painting/controls with indexed conflict selection. Small
/// files retain their individual marker controls and keyboard/hover behavior.
#[allow(clippy::too_many_arguments)]
fn rail(ui: &mut Ui, s: &mut State, id: u64, height: f32, row_height: f32, view: crate::overview::View, widgets: &mut HashMap<String, UiControl>) {
    let (rail, _) = ui.allocate_exact_size(vec2(crate::overview::WIDTH, height.max(24.0)), Sense::hover());
    s.overview_probe = crate::overview::Probe::new(&s.overview, view);
    let overview = crate::overview::show(ui, rail, Id::new(("conflict-overview", id)), &s.overview, view,
        "Amber: unresolved. Green: resolved in this draft. Click a conflict to select it; drag to scroll. Focus here for arrows, Page Up/Down, Home/End.");
    if let Some(o) = &overview {
        register(ui, widgets, "conflict-overview", o.response.rect, o.response.enabled());
        if let Some(target) = o.target {
            s.overview_jump = Some(target);
            ui.ctx().request_repaint();
        }
    } else {
        ui.painter().rect_filled(rail, 3.0, PANEL());
    }
    let total = s.overview.rows.max(1) as f32;
    let per_line = ((rail.height() - 4.0) / total).min(row_height);
    let small = s.overview_open.len() <= RAIL_CONTROL_LIMIT;
    if overview.is_none() {
        // Normally a fitting file has only a few conflicts. Also bound this
        // fallback for transient/degenerate viewport geometry and resolved items.
        let stride = s.overview_spans.len().div_ceil(RAIL_PAINT_LIMIT).max(1);
        for index in (0..s.overview_spans.len()).step_by(stride) {
            let color = match s.items[index].0 {
                Item::Done(_) => GREEN().gamma_multiply(0.7),
                Item::Open(block) if block == s.selected => AMBER(),
                Item::Open(_) => AMBER().gamma_multiply(0.45),
            };
            ui.painter().rect_filled(rail_mark(s, rail, per_line, index), 2.0, color);
        }
    }
    let fallback = (!small && overview.is_none()).then(|| {
        ui.interact(rail, Id::new(("conflict-overview", id)), Sense::click())
    });
    let response = overview.as_ref().map(|o| &o.response).or(fallback.as_ref());
    let mut jump = response.filter(|r| r.clicked()).and_then(|r| r.interact_pointer_pos())
        .and_then(|p| rail_hit(s, rail, per_line, p));
    // Keep existing automation IDs for small files. Dense documents expose the
    // selected marker only; every conflict is still reachable by indexed click
    // or the existing Previous/Next controls.
    if small {
        for &index in &s.overview_open {
            let (Item::Open(block), range) = &s.items[index] else { continue };
            let mark = rail_mark(s, rail, per_line, index);
            if overview.is_none() {
                let r = ui.interact(mark, Id::new(("conflict-marker", id, block)), Sense::click())
                    .on_hover_text(format!("Conflict {} · line {}", index + 1, s.overview_spans[index].0 + 1));
                if r.clicked() { jump = Some((*block, range.start)); }
            }
            register(ui, widgets, &format!("conflict-marker-{block}"), mark, true);
        }
    }
    if let Some(&index) = s.overview_open.get(s.selected) {
        let mark = rail_mark(s, rail, per_line, index);
        ui.painter().rect_stroke(mark, 1.0, Stroke::new(1.0, AMBER()), StrokeKind::Inside);
        if !small { register(ui, widgets, &format!("conflict-marker-{}", s.selected), mark, true); }
    }
    if let Some((block, start)) = jump {
        s.overview_jump = None;
        s.selected = block;
        s.jump = Some(start);
    }
}

/// Room between panes for the bands that join their conflicts.
const BAND_GAP: f32 = 24.0;

/// Bands across the gaps join each conflict's lines in Ours and Theirs to the same
/// conflict in Result: in the side's color while open, green where the resolution used that
/// side, and a dashed outline where it did not.
fn bands(ui: &Ui, s: &State, panes: &[PaneOut; 3]) {
    for (left, right, tint, which) in [(0, 1, BLUE(), 2), (1, 2, VIOLET(), 3)] {
        let (a, b) = (&panes[left], &panes[right]);
        let x = Rangef::new(a.frame.right(), b.frame.left());
        let (top, bottom) = (a.view.top().max(b.view.top()), a.view.bottom().min(b.view.bottom()));
        if x.span() <= 0.0 || bottom <= top {
            continue;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_x_y_ranges(x, Rangef::new(top, bottom)).intersect(ui.clip_rect()));
        for &(k, t0, b0) in &a.rows {
            let Some(&(_, t1, b1)) = b.rows.iter().find(|r| r.0 == k) else { continue };
            if (b0 <= top && b1 <= top) || (t0 >= bottom && t1 >= bottom) {
                continue;
            }
            let style = match s.items.get(k).map(|(item, _)| *item) {
                Some(Item::Open(block)) => {
                    let alpha = if block == s.selected { 0.22 } else { 0.12 };
                    (tint.gamma_multiply(alpha), Stroke::new(1.0, tint.gamma_multiply(0.5)), false)
                }
                Some(Item::Done(r)) if s.resolved.get(r).is_some_and(|r| r.choice == which || r.choice == 0) => {
                    (GREEN().gamma_multiply(0.12), Stroke::new(1.0, GREEN().gamma_multiply(0.35)), false)
                }
                Some(Item::Done(_)) => (Color32::TRANSPARENT, Stroke::new(1.0, MUTED().gamma_multiply(0.5)), true),
                None => continue,
            };
            // Rows scrolled out of view lead to the viewport's edge.
            let clamp = |y: f32| y.clamp(top, bottom);
            band(&painter, x, (clamp(t0), clamp(b0)), (clamp(t1), clamp(b1)), style);
        }
    }
}

/// One band from rows `a` at the left of `x` to rows `b` at its right, its edges eased:
/// its fill, its edge stroke, and whether that edge is dashed.
fn band(painter: &Painter, x: Rangef, a: (f32, f32), b: (f32, f32), (fill, edge, dashed): (Color32, Stroke, bool)) {
    const STEPS: u32 = 12;
    let point = |k: u32, y0: f32, y1: f32| {
        let f = k as f32 / STEPS as f32;
        pos2(lerp(x.min..=x.max, f), lerp(y0..=y1, f * f * (3.0 - 2.0 * f)))
    };
    let top: Vec<Pos2> = (0..=STEPS).map(|k| point(k, a.0, b.0)).collect();
    let bottom: Vec<Pos2> = (0..=STEPS).map(|k| point(k, a.1, b.1)).collect();
    if fill != Color32::TRANSPARENT {
        let mut mesh = epaint::Mesh::default();
        for (upper, lower) in top.iter().zip(&bottom) {
            mesh.colored_vertex(*upper, fill);
            mesh.colored_vertex(*lower, fill);
        }
        for k in 0..STEPS {
            let i = 2 * k;
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i + 1, i + 3, i + 2);
        }
        painter.add(Shape::mesh(mesh));
    }
    for edge_points in [top, bottom] {
        if dashed {
            painter.extend(Shape::dashed_line(&edge_points, edge, 3.0, 3.0));
        } else {
            painter.add(Shape::line(edge_points, edge));
        }
    }
}

fn text_columns(
    ui: &mut Ui,
    s: &mut State,
    id: u64,
    editable: bool,
    names: &[String; 2],
    widgets: &mut HashMap<String, UiControl>,
) {
    s.overview_probe = Default::default();
    if !s.unlinked && let Some(offset) = s.overview_jump.take() {
        s.shared_row = offset / ui.text_style_height(&TextStyle::Monospace);
    }
    let force = s.jump.is_some();
    if let Some(byte) = s.jump.take() {
        // Marker positions are byte boundaries; clamp any stale selection safely.
        let byte = byte.min(s.draft.len());
        let mut byte = byte;
        while !s.draft.is_char_boundary(byte) {
            byte -= 1;
        }
        let line = s.draft[..byte].bytes().filter(|b| *b == b'\n').count() as f32;
        // Two lines above for context, and never past where Result can scroll.
        let row_height = ui.text_style_height(&TextStyle::Monospace);
        s.shared_row = (line - 2.0).max(0.0).min(s.scroll_max[1] / row_height);
        let edit_id = Id::new(("conflict-result", id));
        let mut state = TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(text::CCursorRange::one(text::CCursor::new(
                s.draft[..byte].chars().count(),
            ))));
        state.store(ui.ctx(), edit_id);
    }
    s.three_columns = ui.available_width() >= 850.0 && s.part == 0;
    let mut moved = None;
    if s.three_columns {
        let mut panes: [PaneOut; 3] = Default::default();
        ui.scope(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = BAND_GAP;
            ui.columns(3, |columns| {
                for (index, ui) in columns.iter_mut().enumerate() {
                    ui.spacing_mut().item_spacing.x = spacing;
                    panes[index] = column(ui, s, id, index, editable, force, names, widgets);
                }
            });
        });
        bands(ui, s, &panes);
        moved = panes.iter().rev().find_map(|p| p.moved);
    } else if s.part == 1 {
        source(
            ui,
            s.preview.as_deref(),
            1,
            ("conflict-base", id),
            ui.available_height(),
        );
    } else {
        let index = match s.part {
            2 => 0,
            3 => 2,
            _ => 1,
        };
        moved = column(ui, s, id, index, editable, force, names, widgets).moved;
    }
    if let Some(row) = moved {
        s.shared_row = row;
        ui.ctx().request_repaint();
    }
}
fn source(
    ui: &mut Ui,
    preview: Option<&ConflictPreview>,
    part: u8,
    salt: impl std::hash::Hash + std::fmt::Debug,
    height: f32,
) {
    let text = preview
        .and_then(|p| p.side(part))
        .unwrap_or("This side has no file, or the preview must be reloaded.");
    ScrollArea::both()
        .id_salt(salt)
        .max_height(height)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Labels are selectable and do not copy the entire source on each frame.
            ui.add(
                Label::new(RichText::new(text).monospace())
                    .wrap_mode(TextWrapMode::Extend)
                    .selectable(true),
            );
        });
}
/// One reviewed version: binary or text, size, mode and abbreviated object.
fn describe(version: &ConflictVersion) -> String {
    if version.gitlink { return format!("Submodule commit · {}", &version.id.as_str()[..12]); }
    if let Some(pointer) = &version.lfs {
        return format!("LFS · {} bytes · sha256:{} · {}", pointer.size, &pointer.oid[..12], if version.unavailable.is_some() { "unavailable locally" } else { "verified local object" });
    }
    format!(
        "{} · {}{} · {}",
        if version.binary { "Binary" } else { "Text" },
        size(version.size),
        if version.executable {
            " · executable"
        } else {
            ""
        },
        &version.id.as_str()[..12]
    )
}
fn working_label(review: &WholeConflictPreview) -> String {
    if review.kind() == ConflictKind::Submodule { return "Checkout and all nested files are left unchanged".into(); }
    let Some(bytes) = review.working_size() else {
        return "Absent".into();
    };
    format!(
        "{} · {}{}",
        if review.working_binary() {
            "Binary"
        } else {
            "Text"
        },
        size(bytes),
        if review.working_matches(2) {
            " · holds ours"
        } else if review.working_matches(3) {
            " · holds theirs"
        } else {
            ""
        }
    )
}
fn deleted_by(review: &WholeConflictPreview) -> &'static str {
    match (review.version(2).is_some(), review.version(3).is_some()) {
        (false, false) => "Both sides deleted this file",
        (false, true) => "Ours deleted this file",
        (true, false) => "Theirs deleted this file",
        (true, true) => "Binary file",
    }
}
/// The reviewed confirmation for keeping one whole version.
fn keep_message(review: &WholeConflictPreview, keep: ConflictKeep, unsaved: bool) -> String {
    let (title, name, stage) = match keep {
        ConflictKeep::Ours => ("Keep ours", "our version", Some(2)),
        ConflictKeep::Theirs => ("Keep theirs", "their version", Some(3)),
        ConflictKeep::Deletion => ("Keep deletion", "the deletion", None),
    };
    let kept = stage
        .and_then(|s| review.version(s))
        .map_or_else(|| name.to_owned(), |v| format!("{name} ({})", describe(v)));
    let working = working_label(review);
    let effect = if review.kind() == ConflictKind::Submodule {
        format!("Stage {kept} in the parent repository only. The submodule checkout, local edits and nested files stay unchanged. It may still show an unstaged commit difference afterward. Original index and stages are retained in recovery.")
    } else { match review.planned_change(keep) {
        Some(WholeFileChange::Replaced) => {
            format!("Replace the working file ({working}) with {kept}, then stage only this path.")
        }
        Some(WholeFileChange::Created) => {
            format!("Create the missing working file from {kept}, then stage only this path.")
        }
        Some(WholeFileChange::Removed) => format!(
            "Move the working file ({working}) into recovery, then stage the deletion of only this path."
        ),
        None if keep == ConflictKeep::Deletion => {
            "The working file is already absent; only the deletion of this path is staged.".into()
        }
        None => {
            format!("The working file already holds {kept}; only this path's index entry changes.")
        }
    } };
    let effect = if review.kind() == ConflictKind::Lfs { format!("{effect}\n\nRead the verified local LFS object directly, retain replaced working bytes in recovery, and stage only the exact reviewed pointer. No automatic fetch. The status refresh uses normal Git behavior.") } else { effect };
    format!(
        "{title}: {}\n\n{effect}{}",
        review.path().display_lossy(),
        if unsaved {
            "\n\nYour unsaved conflict-editor text for this file will be discarded."
        } else {
            ""
        }
    )
}
/// Only the choices this conflict offers; each opens a reviewed confirmation.
fn keep_buttons(
    ui: &mut Ui,
    review: &WholeConflictPreview,
    enabled: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    for (key, label, keep) in [
        ("conflict-keep-ours", "Keep ours", ConflictKeep::Ours),
        ("conflict-keep-theirs", "Keep theirs", ConflictKeep::Theirs),
        (
            "conflict-keep-deletion",
            "Keep deletion",
            ConflictKeep::Deletion,
        ),
    ] {
        if review.can_keep(keep) && button(ui, widgets, key, label, enabled) {
            intents.push(Intent::Conflict(ConflictIntent::Keep(keep)));
        }
    }
}
/// Ours or Theirs as a card: its version, or that this side deleted the file.
fn version_card(
    ui: &mut Ui,
    review: &WholeConflictPreview,
    stage: u8,
    caption: &str,
    tint: Color32,
) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, tint.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(caption).font(semibold(11.5)).color(tint));
            match review.version(stage) {
                Some(version) => {
                    ui.label(RichText::new(describe(version)).size(12.5).color(TEXT()));
                    if let Some(failure) = version.unavailable { ui.colored_label(AMBER(), failure.guidance()); }
                    if review.working_matches(stage) {
                        ui.label(
                            RichText::new("The working file holds this version")
                                .size(12.0)
                                .color(GREEN()),
                        );
                    }
                }
                None => {
                    ui.label(
                        RichText::new("Deleted on this side")
                            .size(12.5)
                            .color(AMBER()),
                    );
                }
            }
        });
}
/// Binary or unreadable conflicts: every version, then the available choices.
fn whole_panel(
    ui: &mut Ui,
    review: &WholeConflictPreview,
    enabled: bool,
    id: u64,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let out = ScrollArea::vertical()
        .id_salt(("conflict-whole", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            pages::caption(ui, "Keep one whole version");
            match review.kind() {
                ConflictKind::Submodule => { ui.colored_label(AMBER(), "Index only: keep the chosen commit in the parent. Checkout, local edits and nested files stay intact. Update the checkout separately in Submodules if needed."); }
                ConflictKind::Lfs => { ui.colored_label(MUTED(), "Read a verified local LFS object directly, back up replaced working bytes, then stage its exact pointer. No automatic fetch."); }
                ConflictKind::File => (),
            }
            ui.add_space(6.0);
            let cards = [(2, "OURS · stage 2", BLUE()), (3, "THEIRS · stage 3", VIOLET())];
            if ui.available_width() > 560.0 {
                ui.columns(2, |cols| {
                    for (col, (stage, caption, tint)) in cols.iter_mut().zip(cards) {
                        version_card(col, review, stage, caption, tint);
                    }
                });
            } else {
                for (stage, caption, tint) in cards {
                    version_card(ui, review, stage, caption, tint);
                    ui.add_space(4.0);
                }
            }
            ui.add_space(6.0);
            pages::well(ui, |ui| {
                pages::field(
                    ui,
                    "Base · stage 1",
                    &review
                        .version(1)
                        .map_or_else(|| "None: added on both sides".into(), describe),
                    false,
                );
                pages::field(ui, "Working file", &working_label(review), false);
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| keep_buttons(ui, review, enabled, intents, widgets));
        });
    register(ui, widgets, "conflict-whole-scroll", out.inner_rect, true);
}
/// Conflict editor: path and save actions, the side switch and block choices,
/// Ours/Theirs references when there is room, then the editable result.
#[allow(clippy::too_many_arguments)]
pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    writable: bool,
    writing: Option<bool>,
    drafting: &crate::commit_draft::Settings,
    server: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let id = tab.id;
    let roles = roles(tab.snapshot.as_ref().and_then(|s| s.state.as_ref()));
    let s = &mut tab.conflict;
    if s.ai.open {
        let path = s.path.as_ref().map(|p| p.display_lossy().into_owned());
        let (target, enabled) = (crate::ai_conflict::Target { path: path.as_deref(), preview: s.preview.as_ref(), draft: &s.draft, conflicts: s.blocks.blocks.len() }, writable && writing.is_none());
        crate::ai_conflict::view(ui, &mut s.ai, target, drafting, server, enabled, intents, widgets);
        return;
    }
    s.three_columns = false;
    let editable = writable && !s.running();
    let bound = editable && s.preview.is_some();
    let tall = ui.available_height() > 300.0;
    if !tall {
        ui.spacing_mut().interact_size.y = 24.0;
        ui.spacing_mut().item_spacing.y = 4.0;
    }
    let count = s.blocks.blocks.len();
    if editable && count > 0 {
        let delta = ui.input_mut(|i| {
            if i.consume_key(Modifiers::ALT, Key::ArrowDown) {
                1
            } else if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                -1
            } else {
                0
            }
        });
        if delta != 0 {
            s.selected = (s.selected as isize + delta).rem_euclid(count as isize) as usize;
            s.jump = Some(s.blocks.blocks[s.selected].range.start);
            s.part = 0;
        }
    }
    // From Changes: AI resolve follows the read when this file can use it.
    if s.ai_wanted && !s.running() {
        s.ai_wanted = false;
        if bound && !s.blocks.is_resolved() {
            intents.push(Intent::Conflict(ConflictIntent::Ai(crate::ai_conflict::Action::Open)));
        }
    }
    let whole = s.whole.clone();
    // Text controls apply when the text editor holds the file, or when nothing
    // else does (a failed read keeps the previous draft, as before).
    let text_mode = s.preview.is_some() || (whole.is_none() && s.kept.is_none());
    pages::strip(ui, |ui| {
        // Laid out right to left: the primary save sits outermost.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(cancelling) = writing {
                if button(
                    ui,
                    widgets,
                    "conflict-cancel-save",
                    if cancelling {
                        "Checking cancellation"
                    } else {
                        "Stop saving"
                    },
                    !cancelling,
                ) {
                    intents.push(Intent::Conflict(ConflictIntent::CancelSave));
                }
                ui.spinner();
            } else {
                let r = pages::primary(ui, bound && s.blocks.is_resolved(), "Save and stage");
                register(ui, widgets, "conflict-stage", r.interact_rect, r.enabled());
                if r.clicked() {
                    intents.push(Intent::Conflict(ConflictIntent::Save(true)));
                }
                if button(ui, widgets, "conflict-save", "Save file", bound) {
                    intents.push(Intent::Conflict(ConflictIntent::Save(false)));
                }
            }
            if button(
                ui,
                widgets,
                "conflict-reload",
                "Reload",
                editable && s.path.is_some(),
            ) {
                intents.push(Intent::Conflict(ConflictIntent::Reload(
                    s.path.clone().unwrap(),
                )));
            }
            if button(ui, widgets, "conflict-external", "Merge tool", bound) {
                intents.push(Intent::Tool(crate::external_tools::ToolIntent::Prepare(
                    ExternalToolKind::Merge,
                )));
            }
            let reason = if s.preview.is_none() {
                "AI resolve needs a text conflict"
            } else if s.blocks.is_resolved() {
                "No conflicts left in this file"
            } else {
                "AI resolve is not available right now"
            };
            let r = crate::ai_conflict::resolve_button(ui, bound && !s.blocks.is_resolved()).on_disabled_hover_text(reason);
            register(ui, widgets, "conflict-ai", r.interact_rect, r.enabled());
            if r.clicked() {
                intents.push(Intent::Conflict(ConflictIntent::Ai(crate::ai_conflict::Action::Open)));
            }
            if s.running() && button(ui, widgets, "conflict-cancel", "Stop", true) {
                intents.push(Intent::Conflict(ConflictIntent::Cancel));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let back = pages::ghost(ui, true, "← Changes");
                register(
                    ui,
                    widgets,
                    "conflict-back",
                    back.interact_rect,
                    back.enabled(),
                );
                if back.clicked() {
                    intents.push(Intent::View(View::Changes));
                }
                // A modify/delete conflict has no markers but is not resolved.
                let whole_pill = whole
                    .as_ref()
                    .filter(|w| s.preview.is_none() || w.deleted())
                    .map(|w| {
                        if w.kind() == ConflictKind::Submodule { "Submodule" }
                        else if w.kind() == ConflictKind::Lfs { "Git LFS" }
                        else if w.deleted() {
                            "One side deleted"
                        } else {
                            "Binary"
                        }
                    });
                let (pill, color) = if let Some(keep) = s.kept {
                    (format!("Kept {}", keep.label()), GREEN())
                } else if s.blocks.malformed {
                    ("Markers?".to_owned(), AMBER())
                } else if count > 0 && !s.resolved.is_empty() {
                    (format!("{count} of {} left", count + s.resolved.len()), AMBER())
                } else if count > 0 {
                    (
                        format!("{count} conflict{}", if count == 1 { "" } else { "s" }),
                        RED(),
                    )
                } else if let Some(label) = whole_pill {
                    (label.to_owned(), AMBER())
                } else if s.preview.is_some() {
                    ("Resolved".to_owned(), GREEN())
                } else {
                    (String::new(), FAINT())
                };
                let hover = if s.kept.is_some() {
                    "Resolved by keeping one whole version. Return to Changes to continue."
                        .to_owned()
                } else if whole_pill.is_some() && count == 0 && !s.blocks.malformed {
                    "Git cannot merge this file line by line. Keep one whole version.".to_owned()
                } else {
                    let resolved = match s.resolved.len() {
                        0 => String::new(),
                        n => format!(", {n} resolved here"),
                    };
                    format!(
                        "{count} complete conflict blocks{resolved}{}",
                        if s.blocks.malformed {
                            "; incomplete markers also remain"
                        } else {
                            ""
                        }
                    )
                };
                let pills = if pill.is_empty() {
                    0.0
                } else if pill.chars().count() > 12 {
                    125.0
                } else {
                    90.0
                } + if s.dirty { 70.0 } else { 0.0 };
                ui.allocate_ui(
                    vec2((ui.available_width() - pills - 10.0).max(40.0), 28.0),
                    |ui| match &s.path {
                        Some(path) => {
                            let label = path.display_lossy();
                            let (dir, name) = label
                                .rsplit_once('/')
                                .map_or(("", label.as_ref()), |(d, n)| (d, n));
                            let mut job = text::LayoutJob::default();
                            if !dir.is_empty() {
                                job.append(
                                    &format!("{dir}/"),
                                    0.0,
                                    TextFormat::simple(FontId::proportional(13.0), FAINT()),
                                );
                            }
                            job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                            ui.add(Label::new(job).truncate())
                                .on_hover_text(label.as_ref());
                        }
                        None => {
                            ui.add(
                                Label::new(
                                    RichText::new("Select a conflicted file in Changes")
                                        .color(MUTED()),
                                )
                                .truncate(),
                            );
                        }
                    },
                );
                if !pill.is_empty() {
                    pages::pill(ui, &pill, color).on_hover_text(hover);
                }
                if s.dirty {
                    pages::pill(ui, "Unsaved", AMBER());
                }
                if !tall && let Some(notice) = &s.notice {
                    ui.small("ⓘ").on_hover_text(notice);
                }
            });
        });
    });
    Frame::new()
        .inner_margin(Margin::symmetric(12, if tall { 8 } else { 4 }))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if text_mode {
                ui.horizontal_wrapped(|ui| {
                    let parts = [(0, "Result"), (1, "Base"), (2, "Ours"), (3, "Theirs")];
                    let items: Vec<_> = parts.iter().map(|p| (p.1, s.part == p.0)).collect();
                    for ((part, _), r) in parts.into_iter().zip(pages::segments(ui, &items)) {
                        register(
                            ui,
                            widgets,
                            &format!("conflict-part-{part}"),
                            r.interact_rect,
                            r.enabled(),
                        );
                        if r.clicked() {
                            s.part = part;
                        }
                    }
                    ui.add_space(8.0);
                    for (key, label, delta) in
                        [("conflict-prev", "‹", -1isize), ("conflict-next", "›", 1)]
                    {
                        if delta > 0 {
                            ui.label(
                                RichText::new(if s.blocks.malformed {
                                    "Markers?".into()
                                } else {
                                    // Numbered like the panes' badges, resolved ones included.
                                    let place = s.items.iter().position(|(item, _)| *item == Item::Open(s.selected));
                                    match (place.filter(|_| count > 0), s.items.len()) {
                                        (_, 0) => "0/0".to_owned(),
                                        (Some(place), total) => format!("{}/{total}", place + 1),
                                        (None, total) => format!("–/{total}"),
                                    }
                                })
                                .size(12.0)
                                .color(FAINT()),
                            )
                            .on_hover_text("Conflict block selected in the result");
                        }
                        if button(ui, widgets, key, label, count > 0) {
                            s.selected =
                                (s.selected as isize + delta).rem_euclid(count as isize) as usize;
                            s.jump = Some(s.blocks.blocks[s.selected].range.start);
                            s.part = 0;
                        }
                    }
                    let mut linked = !s.unlinked;
                    let r = ui.checkbox(&mut linked, "Linked scrolling");
                    register(ui, widgets, "conflict-linked", r.interact_rect, r.enabled());
                    if r.changed() {
                        s.unlinked = !linked;
                        s.shared_row = s.scroll_y[1] / ui.text_style_height(&TextStyle::Monospace);
                    }
                });
            }
            // A text modify/delete conflict keeps its editor; the whole-file
            // choices sit beside it. Binary conflicts show them in the body.
            if let Some(w) = whole.as_ref().filter(|_| s.preview.is_some()) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Whole file").font(semibold(12.0)).color(TEXT()));
                    ui.label(RichText::new(deleted_by(w)).size(12.0).color(AMBER()));
                    keep_buttons(ui, w, editable, intents, widgets);
                });
            }
            // With a whole-version review, an unreadable text is guidance, not a failure.
            let error_color = if whole.is_some() && s.preview.is_none() {
                AMBER()
            } else {
                RED()
            };
            if let Some(error) = &s.error {
                if tall {
                    pages::callout(ui, error_color, error);
                } else {
                    ui.add(
                        Label::new(RichText::new(error).size(12.0).color(error_color)).truncate(),
                    )
                    .on_hover_text(error);
                }
            } else if tall && let Some(notice) = &s.notice {
                ui.add(Label::new(RichText::new(notice).size(12.0).color(MUTED())).truncate())
                    .on_hover_text(notice);
            }
            if let Some(error) = &s.whole_error {
                if tall {
                    pages::callout(ui, RED(), error);
                } else {
                    ui.add(Label::new(RichText::new(error).size(12.0).color(RED())).truncate())
                        .on_hover_text(error);
                }
            }
        });
    pages::divider(ui);
    Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 12,
            top: if tall { 10 } else { 4 },
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(keep) = s.kept.filter(|_| s.preview.is_none()) {
                pages::callout(
                    ui,
                    GREEN(),
                    &format!(
                        "Resolved by keeping {}. Return to Changes to continue or abort the operation.",
                        match keep {
                            ConflictKeep::Ours => "ours",
                            ConflictKeep::Theirs => "theirs",
                            ConflictKeep::Deletion => "the deletion",
                        }
                    ),
                );
                return;
            }
            if let Some(w) = whole.as_ref().filter(|_| s.preview.is_none()) {
                whole_panel(ui, w, editable, id, intents, widgets);
                return;
            }
            // Each side named by its marker label and its part in the operation.
            let name = |label: &str, role: &str| if label.is_empty() { role.to_owned() } else { format!("{label} · {role}") };
            let names = [name(&s.labels.0, roles.0), name(&s.labels.1, roles.1)];
            text_columns(ui, s, id, editable, &names, widgets);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn wait(app: &mut Rocket, ctx: &Context) {
        let deadline = Instant::now() + Duration::from_secs(45);
        while app.busy() {
            app.poll(ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
    /// Lay out the whole window on the conflict editor, as the app draws it.
    fn frame(app: &mut Rocket, ctx: &Context) {
        app.tabs[0].view = View::Conflict;
        for _ in 0..3 {
            app.frame_widgets.clear();
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                    ..Default::default()
                },
                |ui| app.shell(ui, &mut Vec::new()),
            );
            output.textures_delta.clear();
        }
    }
    fn path(name: &str) -> RepoPath {
        RepoPath::new(name.as_bytes().to_vec()).unwrap()
    }
    #[test]
    fn binary_and_deleted_conflicts_offer_only_their_whole_versions() {
        common::isolated(
            "ROCKET_CONFLICTS_UI_CASE",
            "conflicts::tests::binary_and_deleted_conflicts_offer_only_their_whole_versions",
            || {
                const BIN: &str = "图像 🚀.bin";
                const DOC: &str = "说明 é.txt";
                let f = common::Fixture::new();
                f.write(BIN, [0, 1, b'b']);
                f.write(DOC, "base\n");
                f.commit("base");
                f.run(&["switch", "-c", "other"]);
                f.write(BIN, [0, 1, b't']);
                fs::remove_file(f.repository.join(DOC)).unwrap();
                f.commit("theirs");
                f.run(&["switch", "main"]);
                f.write(BIN, [0, 1, b'o']);
                f.write(DOC, "ours 界\n");
                f.commit("ours");
                assert!(
                    !f.output(&f.repository, &["merge", "other"])
                        .status
                        .success()
                );
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                let id = app.tabs[0].id;

                // Text modify/delete: the editor keeps the text and offers ours or
                // the deletion, never the missing side.
                app.conflict_intent(ConflictIntent::Open(path(DOC)));
                wait(&mut app, &ctx);
                let s = &app.tabs[0].conflict;
                assert!(s.preview.is_some() && s.whole.as_ref().is_some_and(|w| w.deleted()));
                frame(&mut app, &ctx);
                let widgets = &app.frame_widgets;
                assert!(widgets["conflict-keep-ours"].enabled);
                assert!(widgets["conflict-keep-deletion"].enabled);
                assert!(!widgets.contains_key("conflict-keep-theirs"));
                assert!(widgets.contains_key("conflict-part-0"));
                // Unsaved text is named in the confirmation; cancelling writes nothing.
                app.tabs[0].conflict.draft.push_str("PRIVATE edit\n");
                app.tabs[0].conflict.changed();
                app.conflict_intent(ConflictIntent::Keep(ConflictKeep::Deletion));
                let Some(DialogKind::Confirm(Action::KeepConflictSide { keep, .. }, message)) =
                    app.dialog.as_ref().map(|d| &d.kind)
                else {
                    panic!("no keep confirmation");
                };
                assert_eq!(*keep, ConflictKeep::Deletion);
                assert!(message.contains("Move the working file") && message.contains("discarded"));
                app.dialog = None;
                app.conflict_keep_cancelled(Some(id));
                assert!(
                    app.tabs[0]
                        .conflict
                        .notice
                        .as_ref()
                        .unwrap()
                        .contains("cancelled")
                );
                assert_eq!(
                    fs::read(f.repository.join(DOC)).unwrap(),
                    "ours 界\n".as_bytes()
                );

                // Binary: no text controls; the panel offers ours and theirs only,
                // and binds to this path instead of keeping the other file's text.
                app.load_conflict(id, path(BIN));
                wait(&mut app, &ctx);
                let s = &app.tabs[0].conflict;
                assert_eq!(s.path, Some(path(BIN)));
                assert!(s.preview.is_none() && s.draft.is_empty() && !s.dirty);
                assert!(
                    s.error
                        .as_ref()
                        .unwrap()
                        .contains("unsupported_conflict_file")
                );
                assert!(s.whole.as_ref().is_some_and(|w| w.binary() && !w.deleted()));
                frame(&mut app, &ctx);
                let widgets = &app.frame_widgets;
                assert!(widgets["conflict-keep-ours"].enabled);
                assert!(widgets["conflict-keep-theirs"].enabled);
                assert!(!widgets.contains_key("conflict-keep-deletion"));
                assert!(!widgets.contains_key("conflict-part-0"));
                assert!(!widgets["conflict-save"].enabled && !widgets["conflict-stage"].enabled);
                app.conflict_intent(ConflictIntent::Keep(ConflictKeep::Theirs));
                let Some(DialogKind::Confirm(action, message)) = app.dialog.take().map(|d| d.kind)
                else {
                    panic!("no keep confirmation");
                };
                assert!(
                    message.contains("Replace the working file") && !message.contains("discarded")
                );
                app.execute(id, action);
                wait(&mut app, &ctx);
                let s = &app.tabs[0].conflict;
                assert_eq!(s.kept, Some(ConflictKeep::Theirs));
                assert!(s.whole.is_none() && s.error.is_none(), "{:?}", s.error);
                assert!(s.notice.as_ref().unwrap().contains("Kept theirs"));
                assert_eq!(fs::read(f.repository.join(BIN)).unwrap(), [0, 1, b't']);
                let outcome = &app.activities.last().unwrap().outcome;
                let whole = outcome.whole_conflict.as_ref().unwrap();
                assert!(outcome.command_succeeded && whole.staged);
                assert_eq!(whole.file_change, Some(WholeFileChange::Replaced));
                frame(&mut app, &ctx);
                assert!(!app.frame_widgets.contains_key("conflict-keep-theirs"));
            },
        );
    }
    #[test]
    fn choices_touch_only_selected_utf8_block() {
        let mut s = State { draft: "prefix 界\n<<<<<<< HEAD\nours 🚀\n||||||| base\nold\n=======\ntheirs e\u{301}\n>>>>>>> other\nsuffix\n".into(), ..Default::default() };
        s.changed();
        s.choose(0);
        assert_eq!(s.draft, "prefix 界\nours 🚀\ntheirs e\u{301}\nsuffix\n");
        assert!(s.blocks.is_resolved() && s.dirty);
    }
    #[test]
    fn undo_puts_back_the_exact_conflict_and_selects_it() {
        let original = "prefix 界\n<<<<<<< HEAD\nours 🚀\n||||||| base\nold\n=======\ntheirs e\u{301}\n>>>>>>> other\nmiddle\n<<<<<<< HEAD\nours two\n=======\ntheirs two\n>>>>>>> other\nsuffix\n";
        let mut s = State { draft: original.into(), ..Default::default() };
        s.changed();
        s.selected = 1;
        s.choose(3);
        s.selected = 0;
        s.choose(0);
        assert_eq!(s.draft, "prefix 界\nours 🚀\ntheirs e\u{301}\nmiddle\ntheirs two\nsuffix\n");
        let choices: Vec<_> = s.resolved.iter().map(|r| r.choice).collect();
        assert_eq!(choices, [0, 3]);
        assert!(s.items.iter().all(|(item, _)| matches!(item, Item::Done(_))), "both listed as resolved, in order");
        s.undo(0);
        assert_eq!((s.blocks.blocks.len(), s.selected), (1, 0), "the first conflict is back and selected");
        assert_eq!(&s.draft[s.resolved[0].range.clone()], "theirs two\n", "the later one moved with the text");
        s.undo(0);
        assert_eq!(s.draft, original);
        assert!(s.resolved.is_empty() && s.previous.is_empty());
    }
    #[test]
    fn an_applied_proposal_lands_resolved_and_each_conflict_keeps_its_undo() {
        let block = |n: u8| format!("<<<<<<< HEAD\nours {n}\n=======\ntheirs {n}\n>>>>>>> other\n");
        let original = format!("top\n{}middle\n{}end\n{}", block(1), block(2), block(3));
        let mut s = State { draft: original.clone(), ..Default::default() };
        s.changed();
        // A choice made by hand first stays, and its Undo still fits the text after Apply.
        s.selected = 2;
        s.choose(2);
        s.apply_proposal(vec!["joined 1\nand more\n".into(), "theirs 2\n".into()]);
        assert_eq!(s.draft, "top\njoined 1\nand more\nmiddle\ntheirs 2\nend\nours 3\n");
        assert!(s.blocks.is_resolved() && s.dirty);
        assert_eq!(s.resolved.iter().map(|r| r.choice).collect::<Vec<_>>(), [AI, AI, 2]);
        assert!(s.notice.as_deref().is_some_and(|n| n.starts_with("AI proposal applied")));
        assert_eq!(s.items.len(), 3, "the applied conflicts stay listed, numbered in place");
        // Undo puts one conflict back and leaves the other's text alone.
        s.undo(0);
        assert_eq!((s.blocks.blocks.len(), s.selected), (1, 0));
        assert_eq!(s.draft, format!("top\n{}middle\ntheirs 2\nend\nours 3\n", block(1)));
        s.undo(1);
        assert_eq!(s.draft, format!("top\n{}middle\ntheirs 2\nend\n{}", block(1), block(3)));
        s.undo(0);
        assert_eq!(s.draft, original);
    }
    #[test]
    fn typing_moves_later_resolutions_and_drops_the_one_it_touches() {
        let block = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n";
        let mut s = State { draft: format!("top\n{block}end\n"), ..Default::default() };
        s.changed();
        s.choose(2);
        let text = |s: &State| s.resolved.first().map(|r| s.draft[r.range.clone()].to_owned());
        // Typed above it, even a line starting with its first letter: it moves along.
        s.draft.insert_str(0, "new ");
        s.changed();
        let start = s.resolved[0].range.start;
        s.draft.insert_str(start, "o\n");
        s.changed();
        assert_eq!(text(&s).as_deref(), Some("ours\n"));
        // Typed right after it: it stays, and Undo still puts back the exact block.
        let end = s.resolved[0].range.end;
        s.draft.insert(end, '!');
        s.changed();
        assert_eq!(text(&s).as_deref(), Some("ours\n"));
        s.undo(0);
        assert_eq!(s.draft, format!("new top\no\n{block}!end\n"));
        // Typed inside it: its Undo is gone and the typed text stays.
        s.choose(2);
        let inside = s.resolved[0].range.start + 2;
        s.draft.insert(inside, '?');
        s.changed();
        assert!(s.resolved.is_empty());
        assert_eq!(s.draft, "new top\no\nou?rs\n!end\n");
        // Typed on its first line before its text: markers put back there would not count.
        s.draft = format!("top\n{block}end\n");
        s.changed();
        s.choose(2);
        let start = s.resolved[0].range.start;
        s.draft.insert(start, 'o');
        s.changed();
        assert!(s.resolved.is_empty());
    }
    /// Lays out the result column as frames do and returns its controls and row height.
    fn result_column(s: &mut State) -> (HashMap<String, UiControl>, f32) {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let mut row = 0.0;
        // The first pass only sizes; later passes lay out.
        for _ in 0..3 {
            widgets.clear();
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                row = ui.text_style_height(&TextStyle::Monospace);
                column(ui, s, 1, 1, true, false, &[String::new(), String::new()], &mut widgets);
            });
            output.textures_delta.clear();
        }
        (widgets, row)
    }
    /// A conflict resolved here offers Undo on its first line, no taller than that line.
    #[test]
    fn a_resolved_conflict_offers_undo_on_its_first_line() {
        let mut s = State {
            draft: "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nafter\n".into(),
            ..Default::default()
        };
        s.changed();
        s.choose(3);
        let (widgets, row) = result_column(&mut s);
        let undo = &widgets["conflict-undo-0"];
        assert!(undo.enabled && !widgets.contains_key("conflict-ours-0"));
        // The result's text starts 2 px down; "theirs" is its second line.
        let top = widgets["conflict-result"].rect.top() + 2.0;
        let middle = undo.rect.center().y;
        assert!(undo.rect.height() <= row + 1.0 && middle > top + row && middle < top + 2.0 * row, "Undo at {:?}, rows of {row} px from {top}", undo.rect);
    }
    /// Lays out pane `index` (0 Ours, 1 Result, 2 Theirs) `width` wide and returns, as painted, each
    /// line's rows, the first conflict's control there (Undo or a choice), and its words with the mark.
    fn painted(ctx: &Context, s: &mut State, index: usize, width: f32) -> (Vec<Vec<Rect>>, Rect, Option<Rect>) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0));
        let shown = match index {
            1 => s.draft.clone(),
            _ => s.preview.as_ref().and_then(|p| p.side(if index == 0 { 2 } else { 3 })).unwrap_or_default().to_owned(),
        };
        let key = ["conflict-ours-0", "conflict-undo-0", "conflict-theirs-0"][index];
        let mut widgets = HashMap::new();
        let (mut lines, mut tag) = (Vec::new(), None);
        // The first pass only sizes; later passes lay out.
        for _ in 0..3 {
            widgets.clear();
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                column(ui, s, 1, index, true, false, &[String::new(), String::new()], &mut widgets);
            });
            output.textures_delta.clear();
            (lines, tag) = (Vec::new(), None);
            for clipped in &output.shapes {
                let Shape::Text(text) = &clipped.shape else { continue };
                if text.galley.job.text == shown {
                    let chars: usize = text.galley.rows.iter().map(|row| row.char_count_including_newline().0).sum();
                    assert_eq!(chars, shown.chars().count(), "the cursor counts on every character having a place");
                    lines = text.galley.rows.split_inclusive(|row| row.ends_with_newline)
                        .map(|line| line.iter().map(|row| row.rect().translate(text.pos.to_vec2())).collect())
                        .collect();
                } else if text.galley.job.text.starts_with("Conflict 1") || ["Used", "Not used"].contains(&text.galley.job.text.as_str()) {
                    let words = text.galley.rect.translate(text.pos.to_vec2());
                    tag = Some(words.with_min_x(words.left() - 16.0));
                }
            }
        }
        (lines, widgets.get(key).map_or(Rect::NOTHING, |control| control.rect), tag)
    }
    /// The rows `area` sits on. A row's box runs a pixel into the next, which no ink reaches.
    fn under(lines: &[Vec<Rect>], area: Rect) -> Vec<Rect> {
        lines.iter().flatten().filter(|row| row.width() > 0.0 && area.shrink2(vec2(0.0, 1.5)).intersects(**row)).copied().collect()
    }
    const LONG: &str = "        (self.departure_burn.abs() + self.arrival_burn.abs() + self.plane_burn.abs()) * GRAVITY_LOSS_MARGIN + self.coast_margin(seconds, ROUNDING_ALLOWANCE).abs()\n";
    /// A long line, a conflict of long lines, and a short line.
    fn long_conflict() -> State {
        let mut s = State { draft: format!("{LONG}<<<<<<< HEAD\n{LONG}=======\n{LONG}>>>>>>> other\nafter\n"), ..Default::default() };
        s.changed();
        s
    }
    /// However a resolved line wraps, its tag and Undo sit beside the text and never on it,
    /// from a narrow pane to a wide one, for an AI proposal and for each ordinary choice.
    /// Only the line under them wraps early; the rest of the text keeps the whole width.
    #[test]
    fn a_resolved_conflicts_tag_and_undo_never_cover_wrapped_code() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let (mut covered, mut named, mut tested) = (Vec::new(), 0, 0);
        let (mut unresolved, mut reached) = (long_conflict(), false);
        for choice in [AI, 2, 3, 0] {
            let mut s = long_conflict();
            if choice == AI { s.apply_proposal(vec![LONG.into()]) } else { s.choose(choice) }
            for width in (260..1500).step_by(11) {
                let (lines, undo, tag) = painted(&ctx, &mut s, 1, width as f32);
                assert!(lines.len() >= 4 && !undo.is_negative(), "{width}: lines {lines:?}, Undo {undo:?}");
                tested += 1;
                for row in under(&lines, undo) {
                    covered.push(format!("choice {choice} at {width}: Undo {undo:?} on {row:?}"));
                }
                for row in tag.map(|tag| under(&lines, tag)).unwrap_or_default() {
                    covered.push(format!("choice {choice} at {width}: tag {tag:?} on {row:?}"));
                }
                let line = &lines[1];
                assert!(tag.is_some() || line.iter().all(|row| row.right() + 100.0 > undo.left()), "{width}: a row had room for the name: {line:?}");
                named += usize::from(line.len() > 1 && tag.is_some());
                if choice == AI {
                    assert_eq!(lines[0], painted(&ctx, &mut unresolved, 1, width as f32).0[0], "{width}: the line above moved");
                    reached |= lines[0].iter().any(|row| row.right() > undo.left());
                }
            }
        }
        assert!(covered.is_empty(), "{} overlaps in {tested} layouts:\n{}", covered.len(), covered.join("\n"));
        assert!(named > 0, "a wrapped line with room on a row shows its name");
        assert!(reached, "the line above keeps the width Undo's line gave up");
    }
    /// In Ours and Theirs too nothing sits on wrapped code, in three panes or one: a choice stays
    /// because its line wraps before it, and Used or Not used takes a row with room. The lines
    /// around keep the whole width.
    #[test]
    fn a_side_panes_marks_never_cover_wrapped_code() {
        common::isolated(
            "ROCKET_CONFLICTS_UI_CASE",
            "conflicts::tests::a_side_panes_marks_never_cover_wrapped_code",
            || {
                const DOC: &str = "long.rs";
                let text = |line: &str| format!("{LONG}{line}after\n");
                let f = common::Fixture::new();
                f.write(DOC, text("base\n"));
                f.commit("base");
                f.run(&["switch", "-c", "other"]);
                f.write(DOC, text(&LONG.replace("plane", "theirs")));
                f.commit("theirs");
                f.run(&["switch", "main"]);
                f.write(DOC, text(&LONG.replace("plane", "ours")));
                f.commit("ours");
                assert!(!f.output(&f.repository, &["merge", "other"]).status.success());
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.conflict_intent(ConflictIntent::Open(path(DOC)));
                wait(&mut app, &ctx);
                let s = &mut app.tabs[0].conflict;
                assert_eq!(s.blocks.blocks.len(), 1);
                let (mut covered, mut hidden, mut named, mut reached, mut back, mut around) = (Vec::new(), Vec::new(), 0, false, false, HashMap::new());
                for (three, index, width) in [true, false].into_iter().flat_map(|three| [0, 2].map(|index| (three, index)))
                    .flat_map(|(three, index)| (260..1500).step_by(11).map(move |width| (three, index, width))) {
                    s.three_columns = three;
                    let (lines, choice, tag) = painted(&ctx, s, index, width as f32);
                    assert!(lines.len() == 4 && !choice.is_negative() && tag.is_none(), "{index} at {width}: {lines:?}, {choice:?}, {tag:?}");
                    for row in under(&lines, choice) {
                        covered.push(format!("pane {index} at {width}: choice {choice:?} on {row:?}"));
                    }
                    reached |= lines[0].iter().any(|row| row.right() > choice.left());
                    around.insert((index, width), (lines[0].clone(), choice.left()));
                }
                for choice in [2, 3, 0] {
                    s.selected = 0;
                    s.choose(choice);
                    for (index, width) in [0, 2].into_iter().flat_map(|index| (260..1500).step_by(11).map(move |width| (index, width))) {
                        let (lines, _, tag) = painted(&ctx, s, index, width as f32);
                        for row in tag.map(|tag| under(&lines, tag)).unwrap_or_default() {
                            covered.push(format!("choice {choice}, pane {index} at {width}: {tag:?} on {row:?}"));
                        }
                        let line = &lines[1];
                        if tag.is_none() && line.iter().any(|row| row.right() + 100.0 <= width as f32 - 8.0) {
                            hidden.push(format!("choice {choice}, pane {index} at {width}: {line:?}"));
                        }
                        named += usize::from(line.len() > 1 && tag.is_some());
                        let (above, chip_left) = &around[&(index, width)];
                        assert_eq!(&lines[0], above, "{index} at {width}: the line above moved");
                        back |= line.iter().any(|row| row.right() > *chip_left);
                    }
                    s.undo(0);
                }
                assert!(covered.is_empty() && hidden.is_empty(), "{} overlaps, {} hidden with room on a row:\n{}\n{}", covered.len(), hidden.len(), covered.join("\n"), hidden.join("\n"));
                assert!(named > 0, "a wrapped line with room on a row shows Used or Not used");
                assert!(reached, "the line above keeps the width the choice's line gave up");
                assert!(back, "with its choice gone, a line takes back the whole width");
            },
        );
    }
    /// Laid out in runs, a text that needs no wrapping keeps the rows it has laid out whole,
    /// wherever a run starts: first or last line, a blank line, with or without a final newline.
    #[test]
    fn laying_out_in_runs_keeps_every_row() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let rows = |galley: &Galley| -> Vec<_> {
            galley.rows.iter().map(|row| (row.pos, row.size, row.ends_with_newline, row.glyphs.len())).collect()
        };
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            let font = TextStyle::Monospace.resolve(ui.style());
            for text in ["x", "\n", "\n\n", "a\n", "a\nb", "a\nb\nc\n", "a\n\n\nb\n\n", "\n\na"] {
                let lines = text.split('\n').count();
                let galley = |chip_lines: &[usize]| chip_galley(ui, text, 800.0, chip_lines, 50.0, |text, width| conflict_job(text, &font, width));
                let whole = galley(&[]);
                for first in 0..lines {
                    for undo_lines in [vec![first], vec![first, first], vec![first, first + 1]] {
                        let runs = galley(&undo_lines);
                        assert_eq!(rows(&runs), rows(&whole), "{text:?} with {undo_lines:?}");
                        assert_eq!(runs.text(), text);
                    }
                }
            }
        });
        output.textures_delta.clear();
    }
    /// Below a line that wrapped early the text still matches the screen: a click lands on
    /// the character it hits, and typing goes there.
    #[test]
    fn typing_lands_where_clicked_below_a_line_that_wraps_early() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut s = long_conflict();
        s.choose(2);
        let (lines, ..) = painted(&ctx, &mut s, 1, 420.0);
        let pos = lines[2][0].left_center() + vec2(1.0, 0.0);
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        for event in [Event::PointerMoved(pos), button(true), button(false), Event::Text("X".into())] {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(420.0, 600.0));
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen), events: vec![event], ..Default::default() }, |ui| {
                column(ui, &mut s, 1, 1, true, false, &[String::new(), String::new()], &mut HashMap::new());
            });
            output.textures_delta.clear();
        }
        assert!(s.draft.ends_with("\nXafter\n"), "{:?}", s.draft);
    }
    /// A line that fits keeps Undo and the full words side by side on its row.
    #[test]
    fn a_short_resolved_line_keeps_its_tag_beside_undo() {
        let mut s = State {
            draft: "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nafter\n".into(),
            ..Default::default()
        };
        s.changed();
        s.choose(3);
        let ctx = Context::default();
        crate::style::install(&ctx);
        let (lines, undo, tag) = painted(&ctx, &mut s, 1, 900.0);
        let row = lines[1][0];
        let tag = tag.expect("the whole words fit");
        assert!(tag.width() > 150.0 && (tag.right() - (undo.left() - 6.0)).abs() < 1.0, "tag {tag:?}, Undo {undo:?}");
        assert!((tag.center().y - row.center().y).abs() < 1.0 && (undo.center().y - row.center().y).abs() < 1.0);
        assert!(undo.height() <= row.height() + 1.0, "Undo {undo:?} on {row:?}");
    }
    /// Each conflict's choices sit on its first line and never cover the next one.
    #[test]
    fn conflict_choices_fit_inside_their_marker_row() {
        let mut s = State {
            draft: "before
<<<<<<< HEAD
ours line long enough to reach the choices
=======
theirs
>>>>>>> other
after
".into(),
            ..Default::default()
        };
        s.changed();
        let (widgets, row) = result_column(&mut s);
        let chip = widgets["conflict-ours-0"].rect;
        assert!(chip.height() <= row + 1.0, "choice {chip:?} is taller than a {row} px row");
        assert_eq!(widgets["conflict-ours"].rect, chip, "the selected conflict keeps the old name");
        assert!(widgets["conflict-both-0"].rect.right() > widgets["conflict-ours-0"].rect.right());
    }
    #[test]
    fn overview_follows_unicode_edits_choices_and_undo() {
        let conflict = "<<<<<<< HEAD\nours 文\n=======\ntheirs 🚀\n>>>>>>> topic\n";
        let mut s = State { draft: format!("head\n{conflict}end\n"), ..Default::default() };
        s.changed();
        assert_eq!(s.overview_spans.len(), 1);
        let first = s.overview_spans[0];
        s.draft.insert_str(0, "external 文\n");
        s.changed();
        assert_eq!(s.overview_spans[0], (first.0 + 1, first.1 + 1));
        s.choose(2);
        assert!(matches!(s.items[0].0, Item::Done(_)));
        assert!(s.overview.marked_rows > 0);
        s.undo(0);
        assert!(matches!(s.items[0].0, Item::Open(_)));
        assert_eq!(s.draft, format!("external 文\nhead\n{conflict}end\n"));
        assert_eq!(s.overview_spans[0], (first.0 + 1, first.1 + 1));
        s.forget_resolutions();
        assert!(s.overview_spans.is_empty());
        assert_eq!(s.overview.rows, 0);
    }
    #[test]
    fn a_sides_lines_are_its_whole_lines_nearest_their_place() {
        let side = "a\nx\nb\nx\nc\nax\n";
        assert_eq!(find_lines(side, "x\n", 4), Some(3..4), "the nearer of two copies");
        assert_eq!(find_lines(side, "x\n", 0), Some(1..2));
        assert_eq!(find_lines(side, "b\nx\n", 0), Some(2..4), "several lines");
        assert_eq!(find_lines("ax\n", "x\n", 0), None, "only whole lines count");
        assert_eq!(find_lines(side, "", 0), None, "a side that removed its lines");
        assert_eq!(find_lines(side, "y\n", 0), None);
        let labels = marker_labels("<<<<<<< HEAD\na\n=======\nb\n>>>>>>> topic/x\n");
        assert_eq!(labels, Some(("HEAD".into(), "topic/x".into())));
    }
    /// Beside the side panes, each side's choice sits on that side's lines and Result keeps
    /// Use both; after a choice, a file that fits never scrolls past its end.
    #[test]
    fn three_panes_put_choices_on_their_sides_and_never_scroll_past_the_end() {
        common::isolated(
            "ROCKET_CONFLICTS_UI_CASE",
            "conflicts::tests::three_panes_put_choices_on_their_sides_and_never_scroll_past_the_end",
            || {
                const DOC: &str = "合并 é.txt";
                let text = |first: &str, second: &str| {
                    format!("top\n{first}\nkeep 1\nkeep 2\nkeep 3\nkeep 4\nkeep 5\nkeep 6\n{second}\nend\n")
                };
                let f = common::Fixture::new();
                f.write(DOC, text("base one", "base two"));
                f.commit("base");
                f.run(&["switch", "-c", "other"]);
                f.write(DOC, text("theirs one", "theirs two\ntheirs two more"));
                f.commit("theirs");
                f.run(&["switch", "main"]);
                f.write(DOC, text("ours one", "ours two"));
                f.commit("ours");
                assert!(!f.output(&f.repository, &["merge", "other"]).status.success());
                let ctx = Context::default();
                let mut app =
                    Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
                wait(&mut app, &ctx);
                app.auto_refresh = false;
                app.conflict_intent(ConflictIntent::Open(path(DOC)));
                wait(&mut app, &ctx);
                assert_eq!(app.tabs[0].conflict.blocks.blocks.len(), 2);
                frame(&mut app, &ctx);
                let w = &app.frame_widgets;
                let inside = |control: &str, pane: &str| w[pane].rect.contains(w[control].rect.center());
                assert!(app.tabs[0].conflict.three_columns);
                assert!(inside("conflict-ours-0", "conflict-ours-scroll") && inside("conflict-ours", "conflict-ours-scroll"));
                assert!(inside("conflict-theirs-1", "conflict-theirs-scroll"));
                assert!(inside("conflict-both-0", "conflict-editor-scroll"));
                assert!(!w.contains_key("conflict-ours-1") || inside("conflict-ours-1", "conflict-ours-scroll"));
                // Choosing the first conflict jumps to the second, yet this file fits.
                let s = &mut app.tabs[0].conflict;
                s.selected = 0;
                s.choose(2);
                frame(&mut app, &ctx);
                frame(&mut app, &ctx);
                let s = &app.tabs[0].conflict;
                assert_eq!(s.scroll_y, [0.0; 3], "a file that fits never scrolls past its end");
                assert_eq!(s.labels, ("HEAD".to_owned(), "other".to_owned()));
                assert!(s.sides.len() == 2 && s.sides.iter().all(|l| l.ours.is_some() && l.theirs.is_some()));
                assert!(app.frame_widgets.contains_key("conflict-undo-0"));
            },
        );
    }
}
