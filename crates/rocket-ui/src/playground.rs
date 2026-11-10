use crate::{app::UiControl, backend::Backend, integrations, pages, style::*};
use eframe::egui::*;
use rocket_git::{Error, WriteTask};
use rocket_integrations::{Node, Outcome, playground};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
enum Done {
    Loaded(String, Option<Vec<u8>>),
    Saved(String, Vec<u8>),
    Ran(String, bool, Outcome),
}
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub source: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub save_error: Option<String>,
    saved: String,
    raw: Option<Vec<u8>>,
    loaded: bool,
    task: Option<WriteTask<Done>>,
    stopping: bool,
    saving: bool,
    node: Option<Arc<Node>>,
    values: HashMap<String, Value>,
    state: Value,
    last_source: String,
    request: Option<Value>,
    part: usize,
    reload_confirm: bool,
}
fn message(e: &Error) -> String {
    if let Error::InvalidInput(s) = e {
        s.clone()
    } else {
        crate::backend::user_error(e)
    }
}
impl State {
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn dirty(&self) -> bool {
        self.loaded && self.source != self.saved
    }
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
            self.stopping = true;
        }
    }
    pub fn prepare_exit(&mut self) {
        if !self.saving {
            self.cancel();
        }
    }
    pub fn text_control(&self, target: &str) -> bool {
        fn input(n: &Node, id: &str) -> bool {
            n.id.as_deref() == Some(id) && matches!(n.kind.as_str(), "input" | "multiline")
                || n.children.iter().any(|n| input(n, id))
        }
        target == "int-play-source"
            || target
                .strip_prefix("int-play-ui-")
                .is_some_and(|id| self.node.as_ref().is_some_and(|n| input(n, id)))
    }
    pub fn snapshot(&self) -> Value {
        let mut ids = vec![];
        if let Some(n) = &self.node {
            n.control_ids(&mut ids);
        }
        json!({"open":self.open,"busy":self.running(),"stopping":self.stopping,"loaded":self.loaded,"dirty":self.dirty(),"source_bytes":self.source.len(),"has_preview":self.node.is_some(),"preview_current":self.last_source==self.source,"widget_ids":ids,"offline_request":self.request.as_ref().map(|r|json!({"method":r["method"]})),"error":self.error.as_deref().map(|e|e.split(':').next().unwrap_or("playground_error")),"save_error":self.save_error.as_deref().map(|e|e.split(':').next().unwrap_or("playground_store"))})
    }
    /// Synthetic-fixture opt-in only; never part of the general MCP state.
    pub fn private_snapshot(&self) -> Value {
        fn labels(n: &Node, out: &mut Vec<String>) {
            out.push(n.label.clone());
            for child in &n.children {
                labels(child, out);
            }
        }
        let mut text = vec![];
        if let Some(node) = &self.node {
            labels(node, &mut text);
        }
        json!({"source":self.source,"labels":text,"values":self.values,"state":self.state,"request":self.request})
    }
    fn start(&mut self, task: rocket_git::Result<WriteTask<Done>>) {
        self.error = None;
        self.notice = None;
        self.stopping = false;
        match task {
            Ok(t) => self.task = Some(t),
            Err(e) => self.error = Some(message(&e)),
        }
    }
    pub fn load(&mut self, backend: Backend, directory: PathBuf) {
        if self.running() {
            return;
        }
        self.open = true;
        self.start(WriteTask::spawn(move |_| {
            backend.integration("playground-load", |_| {
                playground::store::load(&directory).map(|(s, r)| Done::Loaded(s, r))
            })
        }));
    }
    fn save(&mut self, backend: Backend, directory: PathBuf) {
        if self.running() || !self.loaded {
            return;
        }
        self.save_error = None;
        self.saving = true;
        let source = self.source.clone();
        let expected = self.raw.clone();
        self.start(WriteTask::spawn(move |cancel| {
            backend.integration("playground-save", |_| {
                playground::store::save(&directory, &source, expected.as_deref(), &cancel)
                    .map(|raw| Done::Saved(source, raw))
            })
        }));
        if self.task.is_none() {
            self.saving = false;
            self.save_error = self.error.clone();
        }
    }
    fn run(&mut self, backend: Backend, event: Option<integrations::PanelEvent>) {
        if self.running() || !self.loaded {
            return;
        }
        if event.is_some() && self.last_source != self.source {
            return;
        }
        let Some(worker) = integrations::worker_path() else {
            self.error = Some("playground_worker: Cannot find Rocket's Lua worker.".into());
            return;
        };
        let reset = event.is_none();
        let source = self.source.clone();
        let row = event.as_ref().and_then(|e| e.row.clone());
        let mut context = json!({"event":event.map_or_else(||"load".into(), |e| e.event),"settings":{},"state":if reset {json!({})} else {self.state.clone()},"values":if reset {json!({})} else {json!(self.values)}});
        if let Some(row) = row {
            context["row"] = json!(row);
        }
        self.start(WriteTask::spawn(move |cancel| {
            backend.integration("playground-run", |client| {
                playground::run(client, &worker, &source, context, &cancel)
                    .map(|out| Done::Ran(source, reset, out))
            })
        }));
    }
    pub fn poll(
        &mut self,
        ctx: &Context,
        backend: Option<Backend>,
        directory: &std::path::Path,
        closing: bool,
    ) {
        if let Some(result) = self.task.as_mut().and_then(WriteTask::try_finish) {
            self.task = None;
            self.stopping = false;
            match result {
                Ok(Done::Loaded(source, raw)) => {
                    self.saved = source.clone();
                    self.source = source;
                    self.raw = raw;
                    self.loaded = true;
                    self.save_error = None;
                }
                Ok(Done::Saved(source, raw)) => {
                    self.saved = source;
                    self.raw = Some(raw);
                    self.notice = Some(crate::i18n::tr("Code saved locally on this device.").into());
                    self.save_error = None;
                }
                Ok(Done::Ran(source, reset, Outcome::View { node, state })) => {
                    if reset {
                        self.values.clear();
                    }
                    node.sync_segments(&mut self.values);
                    self.node = Some(Arc::from(node));
                    self.state = state;
                    self.last_source = source;
                    self.request = None;
                    self.notice = Some(crate::i18n::tr("Preview ready. No network or repository access.").into());
                }
                Ok(Done::Ran(_, _, Outcome::Review(r))) => {
                    self.request = Some(json!({"method":r.method,"path":r.path,"body":r.body}));
                    self.notice =
                        Some(crate::i18n::tr("Script requested HTTP. Preview only; nothing was sent.").into());
                }
                Err(Error::Cancelled) => {
                    self.notice =
                        Some(crate::i18n::tr("Cancelled. Last successful preview and source are retained.").into());
                    if self.saving {
                        self.save_error = Some(
                            "playground_cancelled: Save cancelled; retry before closing.".into(),
                        );
                    }
                }
                Err(e) => {
                    self.error = Some(message(&e));
                    if self.saving {
                        self.save_error = self.error.clone();
                    }
                }
            }
            self.saving = false;
        }
        if closing
            && self.dirty()
            && !self.running()
            && self.save_error.is_none()
            && let Some(backend) = backend
        {
            self.save(backend, directory.to_owned());
        }
        if self.running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}
/// A titled card for one half of the playground; `content` fills its body.
fn panel<R>(ui: &mut Ui, title: &str, detail: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(CARD())
                .corner_radius(CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).font(semibold(13.0)));
                        ui.label(RichText::new(detail).size(12.0).color(HINT()));
                    });
                });
            Frame::new()
                .inner_margin(Margin::symmetric(10, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui)
                })
                .inner
        })
        .inner
}
fn editor(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, height: f32) {
    panel(ui, "scratch.lua", "Lua 5.5", |ui| {
        let area = ScrollArea::both()
            .id_salt("int-play-code")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // A gutter for line numbers, filled once the text is laid out.
                    let (gutter, _) = ui.allocate_exact_size(vec2(30.0, 1.0), Sense::hover());
                    let output = ui
                        .add_enabled_ui(s.loaded && !s.running(), |ui| {
                            TextEdit::multiline(&mut s.source)
                                .font(TextStyle::Monospace)
                                .frame(Frame::new())
                                .desired_width(f32::INFINITY)
                                .desired_rows(24)
                                .char_limit(playground::SOURCE_LIMIT)
                                .show(ui)
                        })
                        .inner;
                    pages::control(widgets, "int-play-source", &output.response, ui.clip_rect());
                    let mut line = 1;
                    let mut starts = true;
                    for row in &output.galley.rows {
                        if starts {
                            let y = output.galley_pos.y + row.rect().center().y;
                            ui.painter().text(
                                pos2(gutter.right(), y),
                                Align2::RIGHT_CENTER,
                                line.to_string(),
                                FontId::monospace(11.5),
                                FAINT(),
                            );
                            line += 1;
                        }
                        starts = row.ends_with_newline;
                    }
                });
            });
        pages::control(
            widgets,
            "int-play-code-scroll",
            &ui.interact(
                area.inner_rect,
                Id::new("int-play-code-scroll"),
                Sense::hover(),
            ),
            ui.clip_rect(),
        );
    });
}
fn preview(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    height: f32,
) -> Option<integrations::PanelEvent> {
    let stale = s.last_source != s.source && s.node.is_some();
    panel(ui, crate::i18n::tr("Preview"), if stale { crate::i18n::tr("Code changed: run to update") } else { crate::i18n::tr("Interactive") }, |ui| {
        let mut event = None;
        let enabled = !s.running() && s.last_source == s.source;
        let area=ScrollArea::both().id_salt("int-play-preview").max_height(height).auto_shrink([false,false]).show(ui,|ui|{
            ui.spacing_mut().item_spacing.y = 10.0;
            if let Some(n)=&s.node {
                Frame::new().fill(BG()).stroke(Stroke::new(1.0, HAIRLINE())).corner_radius(8).inner_margin(Margin::symmetric(14, 12)).show(ui,|ui|{
                    ui.set_width(ui.available_width());
                    let mut changed = false;
                    let mut panel = integrations::Panel::new(&mut s.values, widgets, &mut event, &mut changed, "int-play-ui-").showing(n);
                    integrations::render_node(ui, n, &mut panel, enabled);
                });
            }
            else {pages::empty(ui,crate::i18n::tr("Write Lua and choose Run. Scripts run in an isolated worker with no files, HTTP, credentials or Git access."));}
            if let Some(request)=&s.request {
                pages::caption(ui,crate::i18n::tr("Requested HTTP (not sent)"));
                pages::well(ui,|ui|{ui.add(Label::new(RichText::new(serde_json::to_string_pretty(request).unwrap_or_default()).monospace().size(12.0)).wrap());});
            }
        });
        pages::control(
            widgets,
            "int-play-preview-scroll",
            &ui.interact(
                area.inner_rect,
                Id::new("int-play-preview-scroll"),
                Sense::hover(),
            ),
            ui.clip_rect(),
        );
        event
    })
}
pub fn view(
    ui: &mut Ui,
    s: &mut State,
    backend: Backend,
    directory: PathBuf,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    let idle = !s.running();
    // Header: what this is, and Run as the main action.
    pages::header(ui, Icon::Console, crate::i18n::tr("Lua playground"), crate::i18n::tr("Offline Lua scratchpad · nothing is sent"), |ui| {
        let r = pages::primary(ui, idle && s.loaded, crate::i18n::tr("Run"));
        pages::control(widgets, "int-play-run", &r, ui.clip_rect());
        if r.clicked() {
            s.run(backend.clone(), None);
        }
        if button(
            ui,
            widgets,
            "int-play-save",
            crate::i18n::tr("Save"),
            idle && s.loaded && writable,
        ) {
            s.save(backend.clone(), directory.clone());
        }
        let r = pages::ghost(ui, idle, crate::i18n::tr("Reload saved"));
        pages::control(widgets, "int-play-reload", &r, ui.clip_rect());
        if r.clicked() {
            if s.dirty() {
                s.reload_confirm = true;
            } else {
                s.load(backend.clone(), directory.clone());
            }
        }
        let r = pages::ghost(ui, true, crate::i18n::tr("Extensions"));
        pages::control(widgets, "int-play-back", &r, ui.clip_rect());
        if r.clicked() {
            s.open = false;
            s.cancel();
        }
    });
    Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 12 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if !idle {
                ui.spinner();
                if button(
                    ui,
                    widgets,
                    "int-play-cancel",
                    if s.stopping { crate::i18n::tr("Stopping") } else { crate::i18n::tr("Stop") },
                    !s.stopping,
                ) {
                    s.cancel();
                }
            }
            if s.dirty() {
                pages::pill(ui, crate::i18n::tr("Unsaved code"), AMBER());
            } else {
                pages::pill(ui, crate::i18n::tr("Saved locally"), GREEN());
            }
            if let Some(n) = &s.notice {
                ui.add(Label::new(RichText::new(n).size(12.0).color(HINT())).truncate());
            }
        });
        if s.reload_confirm {
            pages::callout(ui, AMBER(), crate::i18n::tr("Replace this unsaved code with the saved copy?"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if button(ui, widgets, "int-play-reload-cancel", crate::i18n::tr("Keep editing"), true) {
                    s.reload_confirm = false;
                }
                let r = pages::danger(ui, idle, crate::i18n::tr("Reload and discard edits"));
                pages::control(widgets, "int-play-reload-confirm", &r, ui.clip_rect());
                if r.clicked() {
                    s.reload_confirm = false;
                    s.load(backend.clone(), directory.clone());
                }
            });
        }
        if let Some(e) = &s.error {
            pages::callout(ui, RED(), e);
        }
        let height = (ui.available_height() - 60.0).max(100.0);
        let event = if ui.available_width() >= 850.0 {
            let mut event = None;
            ui.columns(2, |cols| {
                editor(&mut cols[0], s, widgets, height);
                event = preview(&mut cols[1], s, widgets, height);
            });
            event
        } else {
            let items = [(crate::i18n::tr("Code"), s.part == 0), (crate::i18n::tr("Preview"), s.part == 1)];
            let ids = ["int-play-tab-code".to_owned(), "int-play-tab-preview".to_owned()];
            ui.horizontal(|ui| {
                if let Some(i) = crate::preferences::choices(ui, widgets, &items, &ids) {
                    s.part = i;
                }
            });
            if s.part == 0 {
                editor(ui, s, widgets, height);
                None
            } else {
                preview(ui, s, widgets, height)
            }
        };
        if let Some(event) = event {
            s.run(backend, Some(event));
        }
    });
}
