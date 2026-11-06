//! Commit message helpers for the Changes composer: the repository's
//! `commit.template` and the messages of recent commits made in Rocket.
use crate::{app::*, backend::user_error, i18n::{tr, trf}, style::*};
use eframe::egui::*;
use rocket_git::*;
use std::collections::HashMap;

/// Recent messages kept per repository, newest first.
pub const RECENT_MESSAGES: usize = 20;
/// Longer messages are not kept in the recent list.
pub const RECENT_MESSAGE_BYTES: usize = 4096;

pub struct Composer {
    /// `None` when no template is configured (or it could not be read).
    pub template: Option<CommitTemplate>,
    pub template_error: Option<String>,
    pub message_error: Option<String>,
    task: Option<ReadTask<Option<CommitTemplate>>>,
    /// Read the template again, e.g. after a commit (configuration may change).
    pub needs_read: bool,
    /// Start an empty draft with the template once the read finishes.
    pub prefill_pending: bool,
    /// Messages of commits made from this composer, newest first.
    pub recent: Vec<String>,
    /// A message draft from Claude Code, when turned on in Preferences.
    pub draft: crate::commit_draft::Draft,
}
impl Default for Composer {
    fn default() -> Self {
        Self {
            template: None,
            template_error: None,
            message_error: None,
            task: None,
            needs_read: true,
            prefill_pending: true,
            recent: Vec::new(),
            draft: Default::default(),
        }
    }
}
impl Composer {
    /// Reject an over-limit edit as a whole; never silently truncate pasted text.
    pub(crate) fn edit_message(&mut self, current: &mut String, next: String) {
        if next.len() > MAX_COMMIT_MESSAGE_BYTES {
            self.message_error = Some(tr("Edit exceeds the 1 MiB commit-message limit. The previous draft is unchanged; keep the pasted text in its source document.").into());
        } else {
            *current = next;
            self.message_error = None;
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// Keep a committed message at the front of the recent list.
    pub fn remember(&mut self, message: &str) {
        let message = message.trim_end();
        if message.trim().is_empty() || message.len() > RECENT_MESSAGE_BYTES {
            return;
        }
        self.recent.retain(|m| m != message);
        self.recent.insert(0, message.to_owned());
        self.recent.truncate(RECENT_MESSAGES);
    }
    /// A committed message: remember it, then start the next draft from the
    /// (possibly changed) template.
    pub fn committed(&mut self, message: Option<&str>) {
        if let Some(message) = message {
            self.remember(message);
        }
        self.needs_read = true;
        self.prefill_pending = true;
    }
}

impl Rocket {
    /// Read each repository's commit template in the background and start an
    /// empty draft with it once. A draft the user typed is never replaced.
    pub(crate) fn poll_composer(&mut self) {
        self.poll_commit_drafts();
        let backend = self.backend.clone();
        for tab in &mut self.tabs {
            let idle = tab
                .snapshot
                .as_ref()
                .and_then(|s| s.state.as_ref())
                .is_some_and(|s| s.is_idle());
            let c = &mut tab.composer;
            if let Some(result) = c.task.as_mut().and_then(ReadTask::try_finish) {
                c.task = None;
                match result {
                    Ok(template) => {
                        c.template = template;
                        c.template_error = None;
                    }
                    Err(error) => {
                        c.template = None;
                        c.template_error = Some(user_error(&error));
                    }
                }
            }
            if c.needs_read
                && c.task.is_none()
                && tab.snapshot.is_some()
                && let Some(backend) = &backend
            {
                c.needs_read = false;
                let backend = backend.clone();
                let root = tab.path.clone();
                match ReadTask::spawn(move |cancel| backend.commit_template(&root, &cancel)) {
                    Ok(task) => c.task = Some(task),
                    Err(error) => c.template_error = Some(user_error(&error)),
                }
            }
            if c.prefill_pending && c.task.is_none() && !c.needs_read && tab.draft_loaded {
                c.prefill_pending = false;
                if tab.draft.is_empty()
                    && idle
                    && let Some(template) = &c.template
                {
                    tab.draft = template.message.clone();
                }
            }
        }
    }
}

/// The composer's Messages menu: start from the commit template or reuse a recent
/// message. Either replaces the current draft; nothing is committed.
pub(crate) fn message_menu(
    ui: &mut Ui,
    draft: &mut String,
    composer: &mut Composer,
    widgets: &mut HashMap<String, UiControl>,
) {
    let menu = ui
        .add(quiet(Button::new(RichText::new(tr("Messages")).size(12.5))))
        .on_hover_text(tr("Start from the commit template or a recent commit message"));
    widgets.insert("composer-messages".into(), (&menu).into());
    Popup::menu(&menu).show(|ui| {
        ui.set_min_width(280.0);
        let mut chosen = None;
        match (&composer.template, &composer.template_error) {
            (Some(template), _) => {
                let r = ui
                    .add_enabled(
                        !template.message.is_empty(),
                        Button::new(tr("Use commit template")),
                    )
                    .on_hover_text(trf!(
                        "Replace the message with the text of {path}",
                        path = template.path.display()
                    ))
                    .on_disabled_hover_text(tr("The commit template has only comment lines."));
                widgets.insert("composer-template".into(), (&r).into());
                if r.clicked() {
                    chosen = Some(template.message.clone());
                }
            }
            (None, Some(error)) => {
                let r = ui
                    .add_enabled(false, Button::new(tr("Commit template unavailable")))
                    .on_disabled_hover_text(error.as_str());
                widgets.insert("composer-template".into(), (&r).into());
            }
            (None, None) => {}
        }
        ui.label(RichText::new(tr("RECENT MESSAGES")).size(11.0).color(FAINT()));
        if composer.recent.is_empty() {
            ui.label(
                RichText::new(tr("Messages of commits made here are listed here."))
                    .size(12.0)
                    .color(FAINT()),
            );
        }
        for (i, message) in composer.recent.iter().enumerate() {
            let subject = message.lines().next().unwrap_or_default();
            let label = if subject.chars().count() > 64 {
                format!("{}…", subject.chars().take(63).collect::<String>())
            } else {
                subject.to_owned()
            };
            let r = ui.button(label).on_hover_text(message.as_str());
            widgets.insert(format!("composer-recent-{i}"), (&r).into());
            if r.clicked() {
                chosen = Some(message.clone());
            }
        }
        if !composer.recent.is_empty() {
            ui.separator();
            let clear = ui.button(tr("Clear recent messages"));
            widgets.insert("composer-recent-clear".into(), (&clear).into());
            if clear.clicked() {
                composer.recent.clear();
                ui.close();
            }
        }
        if let Some(message) = chosen {
            *draft = message;
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_edit_keeps_previous_draft_and_accepts_full_writer_limit() {
        let mut composer = Composer::default();
        let mut draft = "previous text".to_owned();
        composer.edit_message(&mut draft, "x".repeat(MAX_COMMIT_MESSAGE_BYTES + 1));
        assert_eq!(draft, "previous text");
        assert!(composer.message_error.is_some());
        let valid = format!("{}x", "文".repeat(MAX_COMMIT_MESSAGE_BYTES / 3));
        assert_eq!(valid.len(), MAX_COMMIT_MESSAGE_BYTES);
        composer.edit_message(&mut draft, valid.clone());
        assert_eq!(draft, valid);
        assert!(composer.message_error.is_none());
    }

    #[test]
    fn recent_messages_are_newest_first_unique_and_bounded() {
        let mut c = Composer::default();
        c.remember("first\n");
        c.remember("second");
        c.remember("first");
        c.remember("   ");
        c.remember(&"x".repeat(RECENT_MESSAGE_BYTES + 1));
        assert_eq!(c.recent, ["first", "second"]);
        for i in 0..30 {
            c.remember(&format!("message {i}"));
        }
        assert_eq!(c.recent.len(), RECENT_MESSAGES);
        assert_eq!(c.recent[0], "message 29");
        c.committed(Some("final 文"));
        assert_eq!(c.recent[0], "final 文");
        assert!(c.needs_read && c.prefill_pending);
    }
}

/// The composer with a real repository: template prefill, a commit through the
/// backend, and the next draft.
#[cfg(all(test, windows))]
mod flow_tests {
    use crate::{
        app::{Intent, Rocket},
        backend::Action,
        test_common as common,
    };
    use eframe::egui::Context;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn wait(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            app.poll(ctx);
            let reading = app
                .tabs
                .iter()
                .any(|t| t.composer.running() || t.composer.needs_read);
            if !app.busy() && !reading {
                return;
            }
            assert!(Instant::now() < deadline, "reads did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn composer_starts_from_the_template_and_remembers_commits() {
        common::isolated(
            "ROCKET_COMPOSER_UI_CASE",
            "composer::flow_tests::composer_starts_from_the_template_and_remembers_commits",
            || {
                let f = common::Fixture::new();
                f.write("file", "one");
                f.commit("one");
                f.write(".gitmessage 文", "feat: \n\n# Why is it needed?\n");
                f.run(&["config", "commit.template", ".gitmessage 文"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                wait(&ctx, &mut app);
                app.auto_refresh = false;
                assert_eq!(app.tabs[0].draft, "feat:");
                let template = app.tabs[0].composer.template.clone().unwrap();
                assert_eq!(template.guidance, "Why is it needed?");

                // A committed message is remembered; the next draft starts from the
                // template again.
                f.write("file", "two");
                f.run(&["add", "file"]);
                let id = app.tabs[0].id;
                app.tabs[0].draft = "feat: second 文".into();
                app.dispatch(Intent::Execute(
                    id,
                    Action::Commit("feat: second 文".into()),
                ));
                wait(&ctx, &mut app);
                let log = f.run(&["log", "-1", "--format=%B"]);
                assert_eq!(String::from_utf8_lossy(&log).trim_end(), "feat: second 文");
                assert_eq!(app.tabs[0].composer.recent, ["feat: second 文"]);
                assert_eq!(app.tabs[0].draft, "feat:");

                // Text typed while a commit runs is kept, and is not replaced.
                f.write("file", "three");
                f.run(&["add", "file"]);
                app.tabs[0].draft = "fix: third".into();
                app.dispatch(Intent::Execute(id, Action::Commit("fix: third".into())));
                app.tabs[0].draft = "typed meanwhile".into();
                wait(&ctx, &mut app);
                assert_eq!(
                    app.tabs[0].composer.recent,
                    ["fix: third", "feat: second 文"]
                );
                assert_eq!(app.tabs[0].draft, "typed meanwhile");
            },
        );
    }
}
