//! Diff rows for the patch dialogs: a patch's files as Git wrote them, with changed
//! words tinted. Display only; the reviewed bytes stay the source of truth.
use crate::{
    diff::{Kind, Line, Patch},
    style::*,
    word_diff::{self, Highlights},
};
use eframe::egui::*;
use rocket_git::CancellationToken;
use std::{collections::BTreeSet, sync::Arc};

/// Height of one diff row.
const ROW: f32 = 19.0;
/// Width of each line-number column.
const GUTTER: f32 = 44.0;

/// One file of a patch, ready to draw.
pub struct Shown {
    pub label: String,
    pub lines: Arc<Vec<Line>>,
    pub words: Highlights,
    pub added: usize,
    pub removed: usize,
    /// The patch was longer than what's shown.
    pub limited: bool,
}

/// A patch's files with their changed words. Runs on a worker: a large patch takes a
/// moment. A mail's headers before the first file, and its signature, are left out.
pub fn files(bytes: &[u8]) -> Vec<Shown> {
    // format-patch ends with "-- " and Git's version, which would read as a removed line.
    let signature = bytes.windows(5).rposition(|w| w == b"\n-- \n").filter(|&at| {
        let mut rest = bytes[at + 5..].split(|b| *b == b'\n').filter(|l| !l.is_empty());
        rest.next().is_some_and(|version| version[0].is_ascii_digit()) && rest.next().is_none()
    });
    let bytes = signature.map_or(bytes, |at| &bytes[..at + 1]);
    let patch = Patch::parse(bytes);
    let limited = patch.limited;
    patch
        .files
        .into_iter()
        .filter(|f| f.label != "Patch" || f.lines.iter().any(|l| l.kind != Kind::Header))
        .map(|f| Shown {
            limited,
            words: word_diff::compute(&f.lines, &CancellationToken::new()).unwrap_or_default(),
            label: f.label,
            lines: f.lines,
            added: f.added,
            removed: f.removed,
        })
        .collect()
}

/// "@@ lines 12–20" for a hunk header, from its new side, or its old one when a hunk
/// only removes lines and has no new side to name.
fn hunk_label(text: &str) -> String {
    let side = |sign: char| {
        text.split_whitespace().find_map(|f| f.strip_prefix(sign)).map(|range| {
            let mut parts = range.split(',');
            let start: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let count: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            (start, count)
        })
    };
    let new = side('+').filter(|&(_, count)| count > 0).or_else(|| side('-')).or(side('+'));
    match new {
        Some((start, 0 | 1)) => format!("@@ line {start}"),
        Some((start, count)) => format!("@@ lines {start}–{}", start + count - 1),
        None => text.to_owned(),
    }
}

/// Which changes of the hunk starting at `at` the marks choose: all, none or some.
fn hunk_marks(lines: &[Line], at: usize, marks: &BTreeSet<usize>) -> (usize, usize) {
    lines[at + 1..]
        .iter()
        .take_while(|l| l.kind != Kind::Hunk)
        .filter(|l| matches!(l.kind, Kind::Added | Kind::Removed))
        .fold((0, 0), |(chosen, all), l| {
            (chosen + usize::from(marks.contains(&l.raw_index)), all + 1)
        })
}

/// What a stash hunk tag says, and its colour.
fn stash_tag(chosen: usize, all: usize) -> (&'static str, Color32) {
    if chosen == all {
        ("Goes into the stash", BLUE())
    } else if chosen == 0 {
        ("Stays in the file", MUTED())
    } else {
        ("Partly stashed", AMBER())
    }
}

/// `files` as unified rows: old and new line numbers, the sign, and the text with
/// changed words tinted. Each file gets a title row when there are several, or when
/// `titled` (nothing else names it). With `marks` (raw line indexes), a bar shows the
/// chosen changes, the others fade, and hunks are tagged.
pub fn rows(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    files: &[Shown],
    titled: bool,
    marks: Option<&BTreeSet<usize>>,
    height: f32,
) {
    Frame::new()
        .fill(VIEW_BG())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(0, 4))
        .show(ui, |ui| list(ui, id, files, titled, marks, height));
}

/// `rows` for a pane that already has its own frame.
pub fn pane(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, files: &[Shown], titled: bool, height: f32) {
    list(ui, id, files, titled, None, height);
}

fn list(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    files: &[Shown],
    titled: bool,
    marks: Option<&BTreeSet<usize>>,
    height: f32,
) {
    let titled = titled || files.len() > 1;
    let mut items = Vec::new();
    for (f, file) in files.iter().enumerate() {
        if titled {
            items.push((f, None));
        }
        items.extend(
            file.lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.kind != Kind::Header)
                .map(|(i, _)| (f, Some(i))),
        );
    }
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing.y = 0.0;
    if items.is_empty() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("No line changes to show.").size(12.0).color(FAINT()));
        });
        ui.add_space(6.0);
        return;
    }
    ScrollArea::both()
        .id_salt(id)
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW, items.len(), |ui, range| {
            for &(f, line) in &items[range] {
                match line {
                    Some(i) => row(ui, &files[f], i, marks),
                    None => title(ui, &files[f]),
                }
            }
        });
}

/// A file's title row inside a multi-file diff.
fn title(ui: &mut Ui, file: &Shown) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, VIEW_PANEL());
    painter.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{}   +{} −{}", file.label, file.added, file.removed),
        FontId::proportional(12.0),
        TEXT(),
    );
}

fn row(ui: &mut Ui, file: &Shown, i: usize, marks: Option<&BTreeSet<usize>>) {
    let line = &file.lines[i];
    let text_left = GUTTER * 2.0 + 18.0;
    if line.kind == Kind::Hunk {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, CARD());
        let label = painter.layout_no_wrap(hunk_label(&line.text), FontId::monospace(11.5), HINT());
        let label_width = label.size().x;
        painter.galley(pos2(rect.left() + 12.0, rect.center().y - label.size().y / 2.0), label, HINT());
        if let Some(marks) = marks {
            let (chosen, all) = hunk_marks(&file.lines, i, marks);
            let (tag, color) = stash_tag(chosen, all);
            let galley = painter.layout_no_wrap(tag.into(), FontId::proportional(11.5), color);
            let pill = Rect::from_min_size(
                pos2(rect.left() + 24.0 + label_width, rect.center().y - 8.0),
                vec2(galley.size().x + 14.0, 16.0),
            );
            painter.rect_filled(pill, 8.0, color.gamma_multiply(0.14));
            painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, color);
        }
        return;
    }
    let changed = matches!(line.kind, Kind::Added | Kind::Removed);
    let chosen = marks.is_none_or(|m| m.contains(&line.raw_index));
    let fade = if changed && !chosen { 0.5 } else { 1.0 };
    let (tint, word_tint, sign_color) = match line.kind {
        Kind::Added => (GREEN(), GREEN().gamma_multiply(0.32), GREEN()),
        Kind::Removed => (RED(), RED().gamma_multiply(0.32), RED()),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT, FAINT()),
    };
    let ink = if changed { TEXT() } else { MUTED() }.gamma_multiply(fade);
    // The text after the sign, with highlighted words; ranges count the sign.
    let font = FontId::monospace(12.0);
    let text = line.text.get(1..).unwrap_or_default();
    let mut job = text::LayoutJob::default();
    let mut at = 0;
    let words = file.words.spans.get(&i).map(Vec::as_slice).unwrap_or_default();
    for word in words {
        let (start, end) = (word.start.saturating_sub(1).min(text.len()), word.end.saturating_sub(1).min(text.len()));
        if start > at && text.is_char_boundary(start) {
            job.append(&text[at..start], 0.0, TextFormat::simple(font.clone(), ink));
        }
        if end > start && text.is_char_boundary(start) && text.is_char_boundary(end) {
            job.append(
                &text[start..end],
                0.0,
                TextFormat {
                    background: word_tint.gamma_multiply(fade),
                    ..TextFormat::simple(font.clone(), ink)
                },
            );
            at = end;
        }
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, TextFormat::simple(font.clone(), ink));
    }
    let galley = ui.painter().layout_job(job);
    let width = (text_left + galley.size().x + 16.0).max(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    let painter = ui.painter();
    if changed {
        painter.rect_filled(rect, 0.0, tint.gamma_multiply(0.10 * fade));
    }
    if marks.is_some() && changed && chosen {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, ROW)), 0.0, BLUE());
    }
    let number = |n: Option<u64>, right: f32| {
        if let Some(n) = n {
            painter.text(
                pos2(rect.left() + right, rect.center().y),
                Align2::RIGHT_CENTER,
                n.to_string(),
                FontId::monospace(11.0),
                FAINT(),
            );
        }
    };
    number(line.old, GUTTER - 8.0);
    number(line.new, GUTTER * 2.0 - 8.0);
    if changed {
        painter.text(
            pos2(rect.left() + GUTTER * 2.0 + 4.0, rect.center().y),
            Align2::LEFT_CENTER,
            if line.kind == Kind::Added { "+" } else { "−" },
            FontId::monospace(12.0),
            sign_color.gamma_multiply(fade),
        );
    }
    painter.galley(
        pos2(rect.left() + text_left, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_split_a_mail_and_leave_out_its_headers_and_signature() {
        let mail = "From 1 Mon Sep 17 00:00:00 2001\nSubject: [PATCH] Change\n---\n a.txt | 2 +-\n\n\
diff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+2\n\
diff --git a/b 文.txt b/b 文.txt\nnew file mode 100644\n--- /dev/null\n+++ b/b 文.txt\n@@ -0,0 +1 @@\n+new\n-- \n2.45.0\n\n";
        let files = files(mail.as_bytes());
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].added, files[0].removed), (1, 1));
        assert_eq!((files[1].added, files[1].removed), (1, 0), "the signature is not a removed line");
        // "two" → "2": the changed words are marked on both lines.
        assert_eq!(files[0].words.spans.len(), 2);
    }

    #[test]
    fn hunk_labels_and_stash_tags() {
        assert_eq!(hunk_label("@@ -1,5 +2,10 @@ fn"), "@@ lines 2–11");
        assert_eq!(hunk_label("@@ -0,0 +1 @@"), "@@ line 1");
        assert_eq!(hunk_label("@@ -1,3 +0,0 @@"), "@@ lines 1–3", "a removed file has no new side");
        let patch = Patch::parse(b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n-x\n+y\n@@ -9,2 +9,2 @@\n-p\n+q\n");
        let lines = &patch.files[0].lines;
        let hunks: Vec<_> = lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Hunk).map(|(i, _)| i).collect();
        let marks: BTreeSet<usize> = [4, 5, 7].into();
        assert_eq!(stash_tag(hunk_marks(lines, hunks[0], &marks).0, 2).0, "Goes into the stash");
        assert_eq!(hunk_marks(lines, hunks[1], &marks), (1, 2));
        assert_eq!(stash_tag(0, 2).0, "Stays in the file");
    }
}
