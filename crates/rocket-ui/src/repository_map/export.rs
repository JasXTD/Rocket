//! Export as Mermaid: the text of what the map shows, copied or saved as a new file.
use super::*;
use crate::pages::{Part, ReviewHeader};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Export {
    pub text: String,
    /// Save writes `name` into `dir`.
    pub dir: String,
    pub name: String,
    /// What the text holds: nodes and the links between them, as shown.
    pub nodes: usize,
    pub links: usize,
    /// "packages", "folders", "commits" or "nodes", for the dialog's subtitle.
    noun: &'static str,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    task: Option<ReadTask<()>>,
}
impl Export {
    pub fn new(
        text: String,
        dir: String,
        kind: MapKind,
        alternate: bool,
        nodes: usize,
        links: usize,
    ) -> Self {
        Self {
            text,
            dir,
            name: default_name(kind, alternate).into(),
            nodes,
            links,
            noun: match (kind, alternate) {
                (MapKind::Code, true) => "packages",
                (MapKind::Code, false) => "folders",
                (MapKind::History, _) => "commits",
                (MapKind::People, _) => "nodes",
            },
            error: None,
            notice: None,
            stopping: false,
            task: None,
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.task {
            t.cancel();
            self.stopping = true;
        }
    }
    pub fn save(&mut self, ctx: &Context, scope: Option<PathBuf>, log: DiagnosticLog) {
        if self.running() {
            return;
        }
        self.error = None;
        self.notice = None;
        let name = self.name.trim();
        if name.is_empty()
            || name.contains(['/', '\\', ':'])
            || !Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mmd"))
        {
            self.error = Some(tr("Use a file name ending in .mmd.").into());
            return;
        }
        let dir = Path::new(self.dir.trim());
        if !dir.is_absolute() {
            self.error = Some(tr("Choose a folder with Browse, or type its full path.").into());
            return;
        }
        let path = dir.join(name);
        let text = self.text.clone();
        self.stopping = false;
        match crate::read_task::spawn(ctx, move |cancel| {
            log.request("map-export", |_| save(&path, &text, scope.as_deref(), &cancel))
        }) {
            Ok(t) => self.task = Some(t),
            Err(e) => self.error = Some(map_error(&e)),
        }
    }
    pub fn poll(&mut self) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            self.stopping = false;
            match result {
                Ok(()) => self.notice = Some(trf!("Saved {name}", name = self.name.trim())),
                Err(Error::Cancelled) => self.notice = Some(tr("Stopped. No file was made.").into()),
                Err(Error::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    self.error = Some(
                        tr("That file already exists. Choose another name; nothing was replaced.")
                            .into(),
                    )
                }
                Err(e) => self.error = Some(map_error(&e)),
            }
        }
    }
}
/// Where Save puts the file until another folder is chosen.
pub(super) fn default_folder(scope: Option<&Path>) -> String {
    crate::folder_browser::save_folder(scope)
}
pub(super) fn default_name(kind: MapKind, alternate: bool) -> &'static str {
    match (kind, alternate) {
        (MapKind::Code, true) => "rocket-packages.mmd",
        (MapKind::Code, false) => "rocket-folders.mmd",
        (MapKind::History, _) => "rocket-history.mmd",
        (MapKind::People, _) => "rocket-people.mmd",
    }
}
fn save(path: &Path, text: &str, scope: Option<&Path>, cancel: &CancellationToken) -> Result<()> {
    save_before_publish(path, text, scope, cancel, || {})
}
fn save_before_publish(
    path: &Path,
    text: &str,
    scope: Option<&Path>,
    cancel: &CancellationToken,
    before: impl FnOnce(),
) -> Result<()> {
    cancel.check()?;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path
            .file_name()
            .is_none_or(|n| n.to_string_lossy().contains(':'))
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("mmd"))
    {
        return Err(Error::InvalidInput(
            "Choose an absolute path to a new .mmd file in an existing folder.".into(),
        ));
    }
    if text.len() > 8 * 1024 * 1024 {
        return Err(Error::InvalidInput(
            "Diagram exceeds the 8 MiB export limit. Narrow the view.".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidInput("Choose an export folder.".into()))?;
    let _guards = scope
        .map(|root| crate::folder_browser::scope::pin_directory(parent, root))
        .transpose()?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let temporary = parent.join(format!(
        ".rocket-map-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let io = |source| Error::Io {
        action: "save Mermaid to a new file (existing files are never overwritten)",
        source,
    };
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(&temporary).map_err(io)?;
    let result = (|| {
        for bytes in text.as_bytes().chunks(16384) {
            cancel.check()?;
            file.write_all(bytes).map_err(io)?;
        }
        file.sync_all().map_err(io)?;
        before();
        cancel.check()?;
        // Publication is atomic and never replaces a file, even if another app
        // creates it during preparation. A cancellation after publication is success.
        fs::hard_link(&temporary, path).map_err(io)?;
        Ok(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup.map_err(|_|Error::Unsupported("Mermaid file was saved, but its temporary link could not be removed. Do not retry the export to the same filename.".into()))
}


#[derive(Default)]
struct Act {
    copy: bool,
    save: bool,
    browse: bool,
    stop: bool,
}
/// The export dialog over the map.
pub(super) fn dialog(
    ctx: &Context,
    s: &mut MapPage,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(e) = &mut s.export else { return };
    // Translate the full summary, never the machine noun or exported diagram.
    let nodes = pages::number_text(e.nodes);
    let links = pages::number_text(e.links);
    let subtitle = match e.noun {
        "packages" => trf!("In view: packages {nodes} · links {links}", nodes = nodes, links = links),
        "folders" => trf!("In view: folders {nodes} · links {links}", nodes = nodes, links = links),
        "commits" => trf!("In view: commits {nodes} · links {links}", nodes = nodes, links = links),
        _ => trf!("In view: nodes {nodes} · links {links}", nodes = nodes, links = links),
    };
    let mut act = Act::default();
    let response = pages::review_at(
        ctx,
        "map-export-dialog",
        720.,
        ReviewHeader {
            image: Icon::Map,
            title: tr("Export as Mermaid"),
            subtitle: &subtitle,
            danger: false,
        },
        widgets,
        "map-export-body",
        true,
        |ui, part, widgets| match part {
            Part::Body => body(ui, e, widgets, &mut act),
            Part::Footer => footer(ui, e, widgets, &mut act),
        },
    );
    // Automation closes this dialog by its own name, as before it was a dialog.
    if let Some(close) = widgets.get("dialog-close").copied() {
        widgets.insert("map-export-close".into(), close);
    }
    if act.copy {
        ctx.copy_text(e.text.clone());
        e.error = None;
        e.notice = Some(tr("Copied the Mermaid text").into());
    }
    if act.stop {
        e.stop();
    }
    if act.save {
        emit(intents, MapIntent::SaveExport);
    }
    if act.browse {
        emit(intents, MapIntent::BrowseExport);
    }
    // Esc and × stop a save first, then close.
    if response.should_close() {
        if e.running() {
            e.stop();
        } else {
            s.export = None;
        }
    }
}
fn body(ui: &mut Ui, e: &mut Export, widgets: &mut HashMap<String, UiControl>, act: &mut Act) {
    let lines: Vec<&str> = e.text.lines().collect();
    let digits = lines.len().max(1).to_string().len() as f32;
    let row = 18.;
    let frame = Frame::new()
        .fill(WELL())
        .stroke(Stroke::new(1., CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::both()
                .id_salt("map-export-scroll")
                .auto_shrink([false, false])
                .max_height(300.)
                .show_rows(ui, row, lines.len(), |ui, range| {
                    // One text line per row, as tall as show_rows counts them.
                    ui.spacing_mut().item_spacing.y = 0.;
                    ui.spacing_mut().interact_size.y = row;
                    for k in range {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.;
                            ui.add_sized(
                                [digits * 8. + 6., row],
                                Label::new(
                                    RichText::new((k + 1).to_string())
                                        .monospace()
                                        .size(12.)
                                        .color(DIM()),
                                ),
                            );
                            ui.add(
                                Label::new(
                                    RichText::new(lines[k])
                                        .monospace()
                                        .size(12.5)
                                        .color(TEXT_SOFT()),
                                )
                                .extend(),
                            );
                        });
                    }
                })
                .inner_rect
        });
    pages::region(widgets, "map-export-scroll", frame.inner.intersect(ui.clip_rect()), true);
    ui.add_space(6.);
    let busy = e.running();
    let field = (ui.available_width() - 160.).max(120.);
    ui.horizontal(|ui| {
        ui.add_sized([60., CONTROL], Label::new(RichText::new(tr("Save in")).color(HINT())));
        let r = ui.add_enabled(
            !busy,
            TextEdit::singleline(&mut e.dir)
                .id_salt("map-export-folder")
                .char_limit(4096)
                .desired_width(field)
                .hint_text(crate::i18n::tr("Folder")),
        );
        pages::control(widgets, "map-export-folder", &r, ui.clip_rect());
        let r = ui.add_enabled(!busy, Button::new(crate::i18n::tr("Browse")));
        pages::control(widgets, "map-export-browse", &r, ui.clip_rect());
        act.browse |= r.clicked();
    });
    ui.horizontal(|ui| {
        ui.add_sized([60., CONTROL], Label::new(RichText::new(crate::i18n::tr("Name")).color(HINT())));
        let r = ui.add_enabled(
            !busy,
            TextEdit::singleline(&mut e.name)
                .id_salt("map-export-name")
                .char_limit(255)
                .desired_width(field)
                .hint_text(tr("File name.mmd")),
        );
        pages::control(widgets, "map-export-name", &r, ui.clip_rect());
    });
    if let Some(error) = &e.error {
        ui.add(Label::new(RichText::new(error).color(RED())).wrap());
    } else if let Some(notice) = &e.notice {
        ui.add(Label::new(RichText::new(notice).color(GREEN())).wrap());
    }
}
fn footer(ui: &mut Ui, e: &mut Export, widgets: &mut HashMap<String, UiControl>, act: &mut Act) {
    let busy = e.running();
    let ready = !busy && !e.dir.trim().is_empty() && !e.name.trim().is_empty();
    let r = pages::primary(ui, ready, if busy { crate::i18n::tr("Saving") } else { crate::i18n::tr("Save") });
    pages::control(widgets, "map-export-save", &r, ui.clip_rect());
    act.save |= r.clicked();
    if busy {
        let r = ui.add_enabled(
            !e.stopping,
            Button::new(if e.stopping { crate::i18n::tr("Stopping") } else { crate::i18n::tr("Stop") }),
        );
        pages::control(widgets, "map-export-stop", &r, ui.clip_rect());
        act.stop |= r.clicked();
    }
    let r = ui.add_enabled(!busy, Button::new(crate::i18n::tr("Copy")));
    pages::control(widgets, "map-export-copy", &r, ui.clip_rect());
    act.copy |= r.clicked();
    pages::footer_status(ui, busy, tr("Names and paths only, no file contents"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_file_publication_preserves_existing_racing_and_cancelled_destinations() {
        let root = std::env::temp_dir().join(format!(
            "rocket-map-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("結果 🚀.mmd");
        let token = CancellationToken::new();
        save(&path, "flowchart LR\n", Some(&root), &token).unwrap();
        assert!(save(&path, "replace", Some(&root), &token).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "flowchart LR\n");
        let race = root.join("race.mmd");
        assert!(
            save_before_publish(&race, "candidate", Some(&root), &token, || fs::write(
                &race,
                "external edit"
            )
            .unwrap())
            .is_err()
        );
        assert_eq!(fs::read_to_string(&race).unwrap(), "external edit");
        let cancelled = root.join("cancelled.mmd");
        assert!(matches!(
            save_before_publish(&cancelled, "candidate", Some(&root), &token, || token
                .cancel()),
            Err(Error::Cancelled)
        ));
        assert!(!cancelled.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        assert!(
            save(
                &root.join("..\\outside.mmd"),
                "no",
                Some(&root),
                &CancellationToken::new()
            )
            .is_err()
        );
        fs::remove_file(&path).unwrap();
        fs::remove_file(&race).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
