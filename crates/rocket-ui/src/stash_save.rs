//! Save stash: choosing which changed files a partial stash saves.
//!
//! Git records the whole index in a stash, even when it saves only some paths, so
//! a partial stash always includes every staged file; the choice is among the
//! unstaged and untracked changes.
use crate::{app::UiControl, pages, style::*};
use eframe::egui::*;
use rocket_git::*;
use std::collections::HashMap;

/// Files a partial stash can save: changes Git can stash on their own, plus
/// untracked files when they are included.
pub fn candidates(entries: &[StatusEntry], include_untracked: bool) -> Vec<&StatusEntry> {
    entries
        .iter()
        .filter(|e| e.stashable_alone() && (e.kind != EntryKind::Untracked || include_untracked))
        .collect()
}

/// Whether a partial stash is impossible: a staged deletion or rename, or a
/// conflict, can only be stashed together with every change.
pub fn blocked(entries: &[StatusEntry]) -> bool {
    entries.iter().any(|e| e.staged() && !e.stashable_alone())
}

/// Checkboxes for the offered files. Staged files are always selected; selections
/// that are no longer offered (an untracked file after untracked files are
/// excluded) are dropped.
pub(crate) fn checklist(
    ui: &mut Ui,
    entries: &[StatusEntry],
    include_untracked: bool,
    selected: &mut Vec<RepoPath>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if blocked(entries) {
        selected.clear();
        pages::callout(
            ui,
            AMBER(),
            crate::i18n::tr("Selected files are unavailable while a staged deletion, rename or conflict exists. Unstage it in Changes first, or save all changes."),
        );
        return;
    }
    let files = candidates(entries, include_untracked);
    selected.retain(|p| files.iter().any(|e| &e.path == p));
    for entry in files.iter().filter(|e| e.staged()) {
        if !selected.contains(&entry.path) {
            selected.push(entry.path.clone());
        }
    }
    ui.horizontal(|ui| {
        let all = ui.small_button(crate::i18n::tr("Select all"));
        pages::control(widgets, "dialog-stash-all", &all, ui.clip_rect());
        if all.clicked() {
            *selected = files.iter().map(|e| e.path.clone()).collect();
        }
        let none = ui.small_button(crate::i18n::tr("Clear"));
        pages::control(widgets, "dialog-stash-none", &none, ui.clip_rect());
        if none.clicked() {
            selected.retain(|p| files.iter().any(|e| e.staged() && &e.path == p));
        }
    });
    let output = ScrollArea::vertical()
        .id_salt("dialog-stash-files")
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, entry) in files.iter().enumerate() {
                let mut on = selected.contains(&entry.path);
                let mut label = match &entry.original_path {
                    Some(original) => format!(
                        "{} → {}",
                        original.display_lossy(),
                        entry.path.display_lossy()
                    ),
                    None => entry.path.display_lossy().into_owned(),
                };
                if entry.staged() {
                    label.push_str(" · staged");
                }
                let r = ui.add_enabled(!entry.staged(), checkbox(&mut on, label));
                pages::control(
                    widgets,
                    format!("dialog-stash-file-{i}"),
                    &r,
                    ui.clip_rect(),
                );
                if r.changed() {
                    if on {
                        selected.push(entry.path.clone());
                    } else {
                        selected.retain(|p| p != &entry.path);
                    }
                }
            }
        });
    pages::region(
        widgets,
        "dialog-stash-files-scroll",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    pages::note(
        ui,
        &crate::i18n::trf!(
            "Selected files: {selected} / {total}; staged files are always saved.",
            selected = selected.len(),
            total = files.len()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: EntryKind) -> StatusEntry {
        StatusEntry {
            path: RepoPath::new(path.as_bytes().to_vec()).unwrap(),
            original_path: None,
            kind,
            index: Change::Unchanged,
            worktree: Change::Modified,
            submodule: None,
            similarity: None,
        }
    }

    #[test]
    fn untracked_files_are_offered_only_when_included() {
        let mut staged = entry("staged", EntryKind::Tracked);
        staged.index = Change::Modified;
        let entries = [
            entry("tracked", EntryKind::Tracked),
            staged,
            entry("new", EntryKind::Untracked),
            entry("build", EntryKind::Ignored),
        ];
        let names = |include| {
            candidates(&entries, include)
                .iter()
                .map(|e| e.path.display_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(false), ["tracked", "staged"]);
        assert_eq!(names(true), ["tracked", "staged", "new"]);
        assert!(!blocked(&entries));
    }

    #[test]
    fn staged_deletions_renames_and_conflicts_block_a_partial_stash() {
        for (kind, index) in [
            (EntryKind::Tracked, Change::Deleted),
            (EntryKind::RenameOrCopy, Change::Renamed),
            (EntryKind::Unmerged, Change::Unmerged),
        ] {
            let mut held = entry("held", kind);
            held.index = index;
            let entries = [entry("tracked", EntryKind::Tracked), held];
            assert!(blocked(&entries), "{kind:?}");
            assert_eq!(candidates(&entries, true).len(), 1);
        }
        // An unstaged deletion can be stashed alone.
        let mut deleted = entry("deleted", EntryKind::Tracked);
        deleted.worktree = Change::Deleted;
        assert!(!blocked(&[deleted]));
    }
}

#[cfg(test)]
mod flow_tests {
    use crate::{
        app::{DialogKind, Intent, Rocket},
        batch::flow_tests::{click, enabled, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;

    #[test]
    fn partial_stash_always_saves_staged_files() {
        common::isolated(
            "ROCKET_STASH_SAVE_UI_CASE",
            "stash_save::flow_tests::partial_stash_always_saves_staged_files",
            || {
                let f = common::Fixture::new();
                for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
                    f.write(name, "base\n");
                }
                f.commit("base");
                f.write("a.txt", "PRIVATE a\n");
                f.write("b.txt", "PRIVATE b\n");
                f.write("c.txt", "PRIVATE c\n");
                f.run(&["add", "c.txt"]);
                f.run(&["rm", "--quiet", "d.txt"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let selected = |app: &Rocket| {
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .stash_paths
                        .as_ref()
                        .map(|paths| {
                            paths
                                .iter()
                                .map(|p| p.display_lossy().into_owned())
                                .collect::<Vec<_>>()
                        })
                };
                app.dispatch(Intent::Dialog(DialogKind::Stash));
                click(&ctx, &mut app, "dialog-stash-selected");
                // A staged deletion can only be stashed with every change.
                assert_eq!(selected(&app), Some(vec![]));
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                f.run(&["reset", "--quiet", "--", "d.txt"]);
                app.dispatch(Intent::Refresh);
                wait(&ctx, &mut app);
                // Rows: a.txt, b.txt, c.txt (staged: always selected), d.txt.
                assert_eq!(selected(&app), Some(vec!["c.txt".to_owned()]));
                assert!(!enabled(&ctx, &mut app, "dialog-stash-file-2"));
                click(&ctx, &mut app, "dialog-stash-file-0");
                click(&ctx, &mut app, "dialog-stash-none");
                assert_eq!(selected(&app), Some(vec!["c.txt".to_owned()]));
                click(&ctx, &mut app, "dialog-stash-file-0");
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
                assert_eq!(
                    text(f.run(&["diff", "--name-only", "stash@{0}^1", "stash@{0}"])),
                    "a.txt\nc.txt\n"
                );
                assert_eq!(text(f.run(&["diff", "--name-only"])), "b.txt\nd.txt\n");
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
            },
        );
    }
}
