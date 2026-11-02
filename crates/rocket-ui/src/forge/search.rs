//! Bounded local recent searches; these are never credentials or executable queries.
use super::*;
use eframe::egui;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub server: String,
    pub repository: String,
    pub kind: ForgeKind,
    pub state: ForgeState,
    pub search: ForgeSearch,
}
impl Recent {
    pub fn valid(&self) -> bool {
        self.server.len() <= 2048
            && self.repository.len() <= 512
            && !self
                .server
                .chars()
                .chain(self.repository.chars())
                .any(char::is_control)
            && self.search.validate().is_ok()
    }
    pub(super) fn from_state(s: &State) -> Self {
        Self {
            server: s.server.clone(),
            repository: s.repository.clone(),
            kind: if s.section == Section::Issues {
                ForgeKind::Issues
            } else {
                ForgeKind::Pulls
            },
            state: s.state,
            search: s.search.clone(),
        }
    }
}
pub(super) fn remember(s: &mut State, recent: Recent) {
    if !recent.valid() {
        return;
    }
    s.searches.retain(|r| r != &recent);
    s.searches.insert(0, recent);
    s.searches.truncate(12);
}
pub(super) fn controls(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    if !s.provider.is_github() {
        return;
    }
    let mut run = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
        let room = ui.available_width();
        let (author, assignee, recent, search) = (150.0, 164.0, 30.0, 70.0);
        let rest = author + assignee + recent + search + 4.0 * 6.0;
        // Narrow pages give the words a row of their own.
        let words = if room - rest >= 160.0 { room - rest } else { room };
        let r = icon_field(ui, widgets, "hosting-search-text", Icon::Search, "Search titles and bodies on GitHub", &mut s.search.text, words, !busy);
        out.changed |= r.changed();
        run |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        for (key, prefix, value, width) in [
            ("hosting-search-author", "author:", &mut s.search.author, author),
            ("hosting-search-assignee", "assignee:", &mut s.search.assignee, assignee),
        ] {
            let r = prefix_field(ui, widgets, key, prefix, value, width, !busy);
            out.changed |= r.changed();
            run |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        let r = ui
            .add_enabled_ui(!busy, |ui| crate::chrome::icon_button(ui, Icon::Clock, true, false, recent))
            .inner
            .on_hover_text("Recent searches");
        control(widgets, "hosting-search-recent", &r, ui.clip_rect());
        egui::Popup::menu(&r).show(|ui| {
            let context = Recent::from_state(s);
            let choices: Vec<_> = s
                .searches
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.server == context.server && r.repository == context.repository && r.kind == context.kind
                })
                .map(|(i, r)| (i, r.clone()))
                .collect();
            if choices.is_empty() {
                ui.weak("No saved search for this repository");
            }
            for (index, recent) in choices {
                let any = |value: &str| if value.is_empty() { "anyone".to_owned() } else { value.to_owned() };
                let label = format!(
                    "{} · author {} · assignee {} · {:?}",
                    if recent.search.text.is_empty() { "Any text" } else { &recent.search.text },
                    any(&recent.search.author),
                    any(&recent.search.assignee),
                    recent.state
                );
                let r = ui.button(label);
                control(widgets, format!("hosting-search-recent-{index}"), &r, ui.clip_rect());
                if r.clicked() {
                    s.search = recent.search;
                    s.state = recent.state;
                    out.changed = true;
                    ui.close();
                }
            }
            ui.separator();
            let r = ui.add_enabled(!s.searches.is_empty(), Button::new("Clear recent searches"));
            control(widgets, "hosting-search-clear", &r, ui.clip_rect());
            if r.clicked() {
                s.searches.clear();
                ui.close();
            }
        });
        let r = ui.add_enabled(!busy && s.session.is_some(), Button::new("Search"));
        control(widgets, "hosting-search", &r, ui.clip_rect());
        run |= r.clicked();
    });
    if run && !busy && s.session.is_some() {
        out.action = Some("refresh");
    }
    if let Some(total) = s.total {
        ui.weak(format!(
            "{} of {total} matching results loaded",
            s.items.len()
        ));
    }
    if s.incomplete {
        ui.colored_label(AMBER(), "GitHub returned partial results or more than 1,000 matches. Narrow the search or retry.");
    }
}
