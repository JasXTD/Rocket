use super::*;
use crate::tree::{Content, path_label};
use std::collections::BTreeSet;

impl MapPage {
    pub(super) fn reference_sites(
        &self,
        commit: &ObjectId,
        path: &[u8],
    ) -> Option<&BTreeSet<AssetReference>> {
        let data = self.data.as_ref()?;
        if data.tip.as_ref() != Some(commit) {
            return None;
        }
        let index = match data.kind {
            MapKind::Modules => &data.modules.as_ref()?.references,
            MapKind::Code if !self.alternate => {
                let links = self.code.links.as_ref()?;
                if links.tip.as_ref() != Some(commit) {
                    return None;
                }
                &links.references
            }
            _ => return None,
        };
        index.for_file(path)
    }
}

pub struct ReferenceView {
    pub commit: ObjectId,
    pub sites: Vec<AssetReference>,
    pub selected: Option<usize>,
    pub error: Option<String>,
    task: Option<ReadTask<Content>>,
    content: Option<Content>,
    scroll: bool,
}

impl ReferenceView {
    pub(super) fn new(commit: ObjectId, sites: Vec<AssetReference>) -> Self {
        Self {
            commit,
            sites,
            selected: None,
            error: None,
            task: None,
            content: None,
            scroll: false,
        }
    }

    pub fn loading(&self) -> bool {
        self.task.is_some()
    }

    pub fn focused_text(&self) -> Option<&str> {
        let content = self.content.as_ref()?;
        let line = self.sites.get(self.selected?)?.line as usize;
        content
            .lines
            .get(line.checked_sub(content.first)?)
            .map(String::as_str)
    }

    pub(super) fn open(&mut self, index: usize, ctx: &Context, backend: Backend, repo: Repository) {
        let Some(site) = self.sites.get(index) else {
            return;
        };
        let (source, line) = (site.source.clone(), u64::from(site.line));
        let revision = FileSearchSource::Commit(self.commit.clone());
        self.selected = Some(index);
        self.task = None;
        self.content = None;
        self.error = None;
        self.scroll = true;
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.log.request("map-open-reference", |log| {
                let blob = repo
                    .with_log(log.clone())
                    .search_file(&revision, &source, line, &cancel)?;
                crate::tree::prepare_search(blob, line, &HashMap::new(), &cancel)
            })
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }

    pub(super) fn poll(&mut self) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            match result {
                Ok(content) => {
                    self.content = Some(content);
                    if self.focused_text().is_none() {
                        self.error = Some(
                            tr("This reference line is unavailable in the selected commit.").into(),
                        );
                    }
                }
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
    }

    /// Closing this view drops its read and restores the map's selection and zoom.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        intents: &mut Vec<Intent>,
        widgets: &mut HashMap<String, UiControl>,
    ) -> bool {
        let mut close = false;
        pages::strip(ui, |ui| {
            if self.selected.is_some()
                && self.sites.len() > 1
                && button(ui, widgets, "map-reference-list", tr("References"), true)
            {
                self.task = None;
                self.content = None;
                self.error = None;
                self.selected = None;
            }
            ui.strong(tr("Go to reference"));
            pages::pill(ui, &self.commit.as_str()[..12], pages::INFO());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = button(ui, widgets, "map-reference-close", tr("Close"), true);
            });
        });
        if close || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            return true;
        }
        if let Some(index) = self.selected {
            let site = &self.sites[index];
            pages::strip(ui, |ui| {
                ui.label(trf!("Line {line}", line = site.line));
                let path = path_label(&site.source);
                ui.add(Label::new(RichText::new(&path).monospace()).truncate())
                    .on_hover_text(path);
            });
            if let Some(error) = &self.error {
                ui.colored_label(RED(), error);
            } else if let Some(content) = &self.content {
                let focus = (site.line as usize).checked_sub(content.first);
                let scroll = std::mem::take(&mut self.scroll).then_some(focus).flatten();
                crate::tree::source_rows(
                    ui,
                    content,
                    focus,
                    scroll,
                    ("map-source", self.commit.as_str(), site.source.as_ref()),
                    "map-reference-source",
                    widgets,
                );
            } else {
                ui.spinner();
            }
        } else {
            let scroll = ScrollArea::vertical()
                .id_salt("map-reference-sites")
                .show_rows(ui, 30., self.sites.len(), |ui, rows| {
                    for i in rows {
                        let site = &self.sites[i];
                        let path = format!(
                            "{} → {}",
                            path_label(&site.source),
                            path_label(&site.target)
                        );
                        let line = trf!("Line {line}", line = site.line);
                        let info = format!("{} · {line}", site.kind);
                        if details::row(
                            ui,
                            widgets,
                            &format!("map-reference-{i}"),
                            Icon::Link,
                            BLUE(),
                            &path,
                            &info,
                            false,
                            &path,
                        ) {
                            emit(intents, MapIntent::SelectReference(i));
                        }
                    }
                });
            pages::region(
                widgets,
                "map-reference-sites",
                scroll.inner_rect.intersect(ui.clip_rect()),
                true,
            );
        }
        false
    }
}
