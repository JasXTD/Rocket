use super::*;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Export {
    pub text: String,
    pub path: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    task: Option<ReadTask<()>>,
}
impl Export {
    pub fn new(text: String) -> Self {
        Self {
            text,
            path: String::new(),
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
        let path = PathBuf::from(&self.path);
        let text = self.text.clone();
        self.error = None;
        self.notice = None;
        self.stopping = false;
        match crate::read_task::spawn(ctx, move |cancel| {
            log.request("map-export", |_| {
                save(&path, &text, scope.as_deref(), &cancel)
            })
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
                Ok(()) => {
                    self.notice = Some("Mermaid file saved. Existing files were preserved.".into())
                }
                Err(Error::Cancelled) => {
                    self.notice = Some(
                        "Export stopped before publication. No destination file created.".into(),
                    )
                }
                Err(e) => self.error = Some(map_error(&e)),
            }
        }
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

pub(super) fn panel(
    ui: &mut Ui,
    s: &mut MapPage,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(e) = &mut s.export else { return };
    ui.heading("Export Mermaid");
    ui.label(
        "Visible nodes and connections from the pinned snapshot. Review the text before sharing.",
    );
    let mut close = false;
    ui.horizontal_wrapped(|ui| {
        if button(ui, widgets, "map-export-copy", "Copy text", !e.running()) {
            ui.ctx().copy_text(e.text.clone());
            e.notice = Some("Mermaid text copied.".into());
        }
        if button(
            ui,
            widgets,
            "map-export-save",
            "Save new .mmd",
            !e.running() && !e.path.is_empty(),
        ) {
            emit(intents, MapIntent::SaveExport);
        }
        if button(
            ui,
            widgets,
            "map-export-stop",
            if e.stopping {
                "Stopping"
            } else {
                "Stop export"
            },
            e.running() && !e.stopping,
        ) {
            e.stop();
        }
        close = button(ui, widgets, "map-export-close", "Close", !e.running());
    });
    ui.label("New file path · existing files will not be overwritten");
    let r = ui.add_enabled(
        !e.running(),
        TextEdit::singleline(&mut e.path)
            .id_salt("map-export-path")
            .char_limit(4096)
            .desired_width(f32::INFINITY)
            .hint_text("Absolute path ending in .mmd"),
    );
    register(ui, widgets, "map-export-path", &r);
    if let Some(message) = e.error.as_ref().or(e.notice.as_ref()) {
        ui.add(
            Label::new(RichText::new(message).color(if e.error.is_some() {
                RED()
            } else {
                MUTED()
            }))
            .wrap(),
        );
    }
    if e.running() {
        ui.label(if e.stopping {
            "Stopping export…"
        } else {
            "Saving a new file…"
        });
    }
    let mut text = e.text.as_str();
    let scroll = ScrollArea::both()
        .id_salt("map-export-scroll")
        .max_height((ui.available_height() - 10.).max(30.))
        .show(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut text)
                    .font(TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
    widgets.insert(
        "map-export-scroll".into(),
        UiControl {
            rect: scroll.inner_rect.intersect(ui.clip_rect()),
            enabled: true,
        },
    );
    if close {
        s.export = None;
    }
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
