//! Issue-tracker links in commit messages, and their editor in Repository
//! Settings. Rules come from Git config (see `rocket_git::IssueLinkSettings`);
//! a link opens only after Rocket validated its https URL again.
use crate::{app::UiControl, backend::{Backend, Snapshot}, pages, style::*};
use eframe::egui::{
    text::{CCursor, LayoutJob},
    text_selection::LabelSelectionState,
    *,
};
use rocket_git::*;
use std::{collections::HashMap, path::Path, sync::Arc};

// ConfigurationSnapshot can contain credentials. Keep this key opaque and
// in memory, with no Debug/serialization implementation.
#[derive(Clone, PartialEq, Eq)]
struct RuleSource {
    repository: RepositoryInfo,
    configuration: Arc<ConfigurationSnapshot>,
}

/// A tab's compiled rules. They are read when the repository opens, on every
/// manual refresh, and when effective configuration changes (including includes).
#[derive(Default)]
pub struct State {
    pub links: Arc<IssueLinks>,
    task: Option<ReadTask<IssueLinkSettings>>,
    source: Option<RuleSource>,
    pending_source: Option<RuleSource>,
    /// The links of the message shown last: commit, subject and body links.
    cache: Option<(ObjectId, Vec<IssueLink>, Vec<IssueLink>)>,
}

impl State {
    pub(crate) fn reload(&mut self, backend: &Backend, snapshot: &Snapshot, force: bool, ctx: &Context) {
        let source = snapshot.configuration().map(|configuration| RuleSource {
            repository: snapshot.info.clone(), configuration,
        });
        if !force && source.is_some() {
            if self.source == source {
                // A superseded read must not replace the currently correct rules.
                self.task = None;
                self.pending_source = None;
                return;
            }
            if self.task.is_some() && self.pending_source == source { return; }
        }
        // Dropping the task cancels it and disconnects its result. Changed config
        // during a read therefore starts a fresh read instead of being lost.
        self.task = None;
        self.pending_source = None;
        let backend = backend.clone();
        let repository = snapshot.repository.clone();
        if let Ok(task) = crate::read_task::spawn(ctx, move |cancel| backend.issue_link_settings_for(&repository, &cancel)) {
            self.task = Some(task);
            self.pending_source = source;
        }
    }

    /// Take a finished read. A failed read keeps the previous rules.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) else {
            return false;
        };
        self.task = None;
        let source = self.pending_source.take();
        if let Ok(settings) = result {
            self.set(&settings);
            self.source = source; // Failed reads never certify a new cache key.
        }
        true
    }

    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    pub(crate) fn set(&mut self, settings: &IssueLinkSettings) {
        self.task = None; // An editor save also supersedes any in-flight read.
        self.pending_source = None;
        self.source = None;
        self.links = Arc::new(settings.links());
        self.cache = None;
    }

    /// Issue links in a commit's subject and body, found once per commit.
    pub(crate) fn message(
        &mut self,
        id: &ObjectId,
        subject: &str,
        body: &str,
    ) -> (&[IssueLink], &[IssueLink]) {
        if self.cache.as_ref().is_none_or(|(cached, ..)| cached != id) {
            self.cache = Some((id.clone(), self.links.find(subject), self.links.find(body)));
        }
        let (_, subject, body) = self.cache.as_ref().unwrap();
        (subject, body)
    }
}

/// `text` as one wrapped, selectable label whose issue IDs are drawn and act
/// as links. Each link's first row is published as `issue-link-<n>`, numbered
/// from `first`. Returns the label's response and the URL of a link clicked
/// this frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn label(
    ui: &mut Ui,
    text: &str,
    links: &[IssueLink],
    font: FontId,
    color: Color32,
    sense: Sense,
    widgets: &mut HashMap<String, UiControl>,
    first: usize,
) -> (Response, Option<String>) {
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(font.clone(), color);
    let linked = TextFormat {
        underline: Stroke::new(1.0, BLUE().gamma_multiply(0.7)),
        ..TextFormat::simple(font, BLUE())
    };
    let mut at = 0;
    for link in links {
        job.append(&text[at..link.range.start], 0.0, plain.clone());
        job.append(&text[link.range.clone()], 0.0, linked.clone());
        at = link.range.end;
    }
    job.append(&text[at..], 0.0, plain);
    let (pos, galley, response) = Label::new(job)
        .wrap()
        .sense(sense | Sense::click())
        .layout_in_ui(ui);
    LabelSelectionState::label_text_selection(
        ui,
        &response,
        pos,
        galley.clone(),
        color,
        Stroke::NONE,
    );
    let chars = |byte: usize| text[..byte].chars().count();
    let under = response.hover_pos().and_then(|p| char_at(&galley, p - pos));
    let mut hovered = None;
    for (i, link) in links.iter().enumerate() {
        let (start, end) = (chars(link.range.start), chars(link.range.end));
        // The link's first row: to its end, or only its first character when
        // it wraps onto the next row.
        let a = galley.pos_from_cursor(CCursor::new(start));
        let mut b = galley.pos_from_cursor(CCursor::new(end));
        if (b.top() - a.top()).abs() > 1.0 {
            b = galley.pos_from_cursor(CCursor::new(start + 1));
        }
        let first_row = Rect::from_min_max(a.min, pos2(b.min.x.max(a.min.x + 2.0), a.max.y))
            .translate(pos.to_vec2());
        pages::region(
            widgets,
            &format!("issue-link-{}", first + i),
            first_row.intersect(ui.clip_rect()),
            true,
        );
        if under.is_some_and(|c| (start..end).contains(&c)) {
            hovered = Some(link);
        }
    }
    let Some(link) = hovered else {
        return (response, None);
    };
    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    let clicked = response.clicked().then(|| link.url.clone());
    (response.on_hover_text(&link.url), clicked)
}

/// The character drawn under `p` (relative to the galley), if any.
fn char_at(galley: &Galley, p: Vec2) -> Option<usize> {
    let cursor = galley.cursor_from_pos(p).index.0;
    // The nearest cursor is before or after the character under the pointer.
    [cursor.checked_sub(1), Some(cursor)]
        .into_iter()
        .flatten()
        .find(|&k| {
            let a = galley.pos_from_cursor(CCursor::new(k));
            let b = galley.pos_from_cursor(CCursor::new(k + 1));
            // A character that ends its row has no same-row neighbour after it.
            let right = if (b.top() - a.top()).abs() < 1.0 {
                b.min.x
            } else {
                a.min.x + 8.0
            };
            (a.top()..a.bottom()).contains(&p.y) && (a.min.x..right).contains(&p.x)
        })
}

/// The Identity / Issue links switch at the top of Repository Settings.
pub(crate) fn pages(
    ui: &mut Ui,
    editor: &mut Option<Editor>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(editor) = editor else {
        return;
    };
    let tabs = pages::segments(
        ui,
        &[("Identity", !editor.shown), ("Issue links", editor.shown)],
    );
    pages::control(
        widgets,
        "dialog-settings-identity",
        &tabs[0],
        ui.clip_rect(),
    );
    pages::control(
        widgets,
        "dialog-settings-issue-links",
        &tabs[1],
        ui.clip_rect(),
    );
    if tabs[0].clicked() {
        editor.shown = false;
    }
    if tabs[1].clicked() {
        editor.shown = true;
    }
}

/// The Issue links page of Repository Settings.
pub struct Editor {
    /// This page is shown instead of the identity page.
    pub shown: bool,
    pub task: Option<ReadTask<IssueLinkSettings>>,
    /// What was read: saving checks that it is still current.
    pub settings: Option<IssueLinkSettings>,
    pub rules: Vec<IssueLinkRule>,
    pub sample: String,
    pub error: Option<String>,
}

impl Editor {
    pub(crate) fn start(backend: Option<&Backend>, root: Option<&Path>) -> Self {
        let mut editor = Self {
            shown: false,
            task: None,
            settings: None,
            rules: Vec::new(),
            sample: String::new(),
            error: None,
        };
        match (backend, root) {
            (Some(backend), Some(root)) => {
                let backend = backend.clone();
                let root = root.to_path_buf();
                match ReadTask::spawn(move |cancel| backend.issue_link_settings(&root, &cancel)) {
                    Ok(task) => editor.task = Some(task),
                    Err(error) => editor.error = Some(crate::backend::user_error(&error)),
                }
            }
            _ => editor.error = Some("Open a repository first".into()),
        }
        editor
    }

    /// Take a finished read; returns it so the tab can use the same rules.
    pub(crate) fn poll(&mut self) -> Option<IssueLinkSettings> {
        let result = self.task.as_mut().and_then(ReadTask::try_finish)?;
        self.task = None;
        match result {
            Ok(settings) => {
                self.rules = settings.local.clone();
                self.settings = Some(settings.clone());
                Some(settings)
            }
            Err(error) => {
                self.error = Some(crate::backend::user_error(&error));
                None
            }
        }
    }

    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    /// The first problem that prevents saving, if any.
    pub fn problem(&self) -> Option<String> {
        if self.rules.len() > MAX_ISSUE_LINK_RULES {
            return Some(format!("At most {MAX_ISSUE_LINK_RULES} rules"));
        }
        for (i, rule) in self.rules.iter().enumerate() {
            if let Err(error) = rule.validate() {
                return Some(crate::backend::user_error(&error));
            }
            if self.rules[..i]
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case(&rule.name))
            {
                return Some(format!("Two rules are named {}", rule.name));
            }
        }
        None
    }

    pub fn changed(&self) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|s| s.local != self.rules)
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) {
        if self.task.is_some() {
            pages::waiting(ui, "Loading issue link rules…");
            return;
        }
        let Some(settings) = &self.settings else {
            return;
        };
        let mut remove = None;
        for (i, rule) in self.rules.iter_mut().enumerate() {
            pages::well(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").size(12.0).color(FAINT()));
                    let name = ui.add(
                        TextEdit::singleline(&mut rule.name)
                            .char_limit(32)
                            .hint_text("jira")
                            .desired_width(140.0),
                    );
                    pages::control(
                        widgets,
                        format!("dialog-issue-name-{i}"),
                        &name,
                        ui.clip_rect(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = ui.button("Remove");
                        pages::control(
                            widgets,
                            format!("dialog-issue-remove-{i}"),
                            &r,
                            ui.clip_rect(),
                        );
                        if r.clicked() {
                            remove = Some(i);
                        }
                    });
                });
                ui.label(RichText::new("ID pattern").size(12.0).color(FAINT()));
                let pattern = ui.add(
                    TextEdit::singleline(&mut rule.pattern)
                        .font(TextStyle::Monospace)
                        .char_limit(256)
                        .hint_text(r"\b[A-Z][A-Z0-9]+-\d+\b")
                        .desired_width(f32::INFINITY),
                );
                pages::control(
                    widgets,
                    format!("dialog-issue-pattern-{i}"),
                    &pattern,
                    ui.clip_rect(),
                );
                ui.label(RichText::new("Link").size(12.0).color(FAINT()));
                let url = ui.add(
                    TextEdit::singleline(&mut rule.url)
                        .font(TextStyle::Monospace)
                        .char_limit(2048)
                        .hint_text("https://example.atlassian.net/browse/{id}")
                        .desired_width(f32::INFINITY),
                );
                pages::control(
                    widgets,
                    format!("dialog-issue-url-{i}"),
                    &url,
                    ui.clip_rect(),
                );
                if let Err(error) = rule.validate() {
                    ui.label(
                        RichText::new(crate::backend::user_error(&error))
                            .size(12.0)
                            .color(AMBER()),
                    );
                }
            });
        }
        if let Some(i) = remove {
            self.rules.remove(i);
        }
        let add = ui.add_enabled(
            self.rules.len() < MAX_ISSUE_LINK_RULES,
            Button::new("Add rule"),
        );
        pages::control(widgets, "dialog-issue-add", &add, ui.clip_rect());
        if add.clicked() {
            self.rules.push(IssueLinkRule {
                name: format!("rule{}", self.rules.len() + 1),
                pattern: String::new(),
                url: String::new(),
            });
        }
        pages::note(
            ui,
            "Pattern: a regular-expression subset; (?i) ignores case. Link: an https URL with {id} or {1}…{9}.",
        );
        if !settings.inherited.is_empty() {
            pages::caption(ui, "Also applied · from other Git config");
            pages::well(ui, |ui| {
                for (scope, rule) in &settings.inherited {
                    let from = match scope {
                        IssueLinkScope::Repository => "included file",
                        IssueLinkScope::Worktree => "worktree config",
                        IssueLinkScope::Global => "global config",
                        IssueLinkScope::System => "system config",
                    };
                    pages::field(
                        ui,
                        &rule.name,
                        &format!("{} → {} ({from})", rule.pattern, rule.url),
                        true,
                    );
                }
            });
            pages::note(
                ui,
                "A rule here with the same name replaces an inherited one.",
            );
        }
        for problem in settings.problems.iter().filter(|p| {
            !settings
                .local
                .iter()
                .any(|r| p.contains(&format!("Rule {}:", r.name)))
        }) {
            pages::callout(ui, AMBER(), problem);
        }
        pages::caption(ui, "Try it");
        let sample = ui.add(
            TextEdit::singleline(&mut self.sample)
                .hint_text("Paste a commit message, e.g. PROJ-12: fix #7")
                .desired_width(f32::INFINITY),
        );
        pages::control(widgets, "dialog-issue-sample", &sample, ui.clip_rect());
        if !self.sample.is_empty() {
            let links = IssueLinks::new(
                self.rules
                    .iter()
                    .chain(settings.inherited.iter().map(|(_, r)| r)),
            );
            let found = links.find(&self.sample);
            if found.is_empty() {
                pages::note(ui, "No issue IDs found.");
            }
            for link in found {
                pages::field(ui, &self.sample[link.range.clone()], &link.url, true);
            }
        }
        if let Some(error) = &self.error {
            pages::callout(ui, RED(), error);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, frame, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::IssueLinkRule;

    /// A configured rule links an ID in the selected commit's subject; a rule added
    /// in Repository Settings is saved to the repository config and links the body.
    #[test]
    fn rules_link_commit_ids_and_the_editor_saves_new_rules() {
        common::isolated(
            "ROCKET_ISSUE_LINKS_UI_CASE",
            "issue_links::tests::rules_link_commit_ids_and_the_editor_saves_new_rules",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "a");
                f.commit("PROJ-12: fix the crash\n\nRefs #7");
                f.run(&["config", "rocket.issuelink.jira.pattern", r"\b[A-Z]+-\d+\b"]);
                f.run(&[
                    "config",
                    "rocket.issuelink.jira.url",
                    "https://jira.example.com/browse/{id}",
                ]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.opened_urls = Some(Vec::new());
                assert_eq!(app.tabs[0].issue_links.links.len(), 1);
                click(&ctx, &mut app, "issue-link-0");
                assert_eq!(
                    app.opened_urls.as_deref().unwrap(),
                    ["https://jira.example.com/browse/PROJ-12"]
                );
                assert!(!app.frame_widgets.contains_key("issue-link-1"));

                app.dispatch(Intent::Dialog(DialogKind::RepositorySettings));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-settings-issue-links");
                let editor = app.dialog.as_mut().unwrap().issue_links.as_mut().unwrap();
                assert_eq!(editor.rules.len(), 1);
                editor.rules.push(IssueLinkRule {
                    name: "redmine".into(),
                    pattern: r"#(\d+)".into(),
                    url: "https://redmine.example.com/issues/{1}".into(),
                });
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert!(app.dialog.is_none());
                let config = std::fs::read_to_string(f.repository.join(".git/config")).unwrap();
                assert!(
                    config.contains("[rocket \"issuelink.redmine\"]"),
                    "{config}"
                );
                assert_eq!(app.tabs[0].issue_links.links.len(), 2);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "issue-link-1");
                assert_eq!(
                    app.opened_urls.as_deref().unwrap()[1],
                    "https://redmine.example.com/issues/7"
                );
            },
        );
    }
}
