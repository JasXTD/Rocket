//! Debounced, cancellable repository searches. Git and text matching stay on workers.
use crate::{
    app::{Rocket, View},
    backend::user_error,
};
use eframe::egui::Context;
use rocket_git::*;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub kind: HistorySearchKind,
    /// How a Changed code search matches; ignored by the other kinds.
    pub content: ContentMatch,
    pub commits: Vec<Commit>,
    pub next: Option<HistorySearchCursor>,
    pub scanned: usize,
    pub complete: bool,
    pub paused: bool,
    /// Paused by Stop, rather than by an error.
    pub stopped: bool,
    pub error: Option<String>,
    /// `error` is Git refusing the regular expression.
    pub bad_pattern: bool,
    /// The path couldn't be used, so nothing ran and nothing can resume.
    pub refused: bool,
    pub task: Option<ReadTask<HistorySearchPage>>,
    pub due: Option<Instant>,
    target: usize,
}
impl SearchState {
    /// Start over with this query. `content` is the Changed code options, or why
    /// the typed path can't be used; then nothing runs and the reason shows.
    pub fn reset(
        &mut self,
        query: &str,
        kind: HistorySearchKind,
        content: std::result::Result<ContentMatch, String>,
    ) {
        // Dropping the old receiver cancels it and rejects any late result.
        *self = Self {
            query: query.trim().into(),
            kind,
            target: 200,
            ..Default::default()
        };
        if self.query.is_empty() {
            return;
        }
        match content {
            Ok(content) => {
                self.content = content;
                self.due = Some(Instant::now() + Duration::from_millis(300));
            }
            // Only Changed code reads the path; the other kinds run without it.
            Err(_) if kind != HistorySearchKind::Content => {
                self.due = Some(Instant::now() + Duration::from_millis(300));
            }
            Err(reason) => {
                self.error = Some(reason);
                self.paused = true;
                self.refused = true;
            }
        }
    }
    pub fn scanning(&self) -> bool {
        self.task.is_some() || self.due.is_some()
    }
    pub fn stop(&mut self) {
        self.stopped = self.scanning() && !self.complete;
        self.task = None;
        self.due = None;
        self.paused = !self.complete;
    }
    pub fn more(&mut self) {
        if self.scanning() || self.complete || self.bad_pattern || self.refused {
            return;
        }
        self.paused = false;
        self.stopped = false;
        self.error = None;
        self.target = self.commits.len().saturating_add(200);
        self.due = Some(Instant::now());
    }
}

/// The path typed in a search's path field: exact, `\` read as `/`, outer slashes
/// and spaces dropped. Empty means the whole repository.
pub(crate) fn search_path(text: &str) -> std::result::Result<Option<RepoPath>, String> {
    let text = text.trim().replace('\\', "/");
    let text = text.trim_matches('/');
    let text = text.strip_prefix("./").unwrap_or(text);
    if text.is_empty() {
        return Ok(None);
    }
    RepoPath::new(text.as_bytes().to_vec())
        .map(Some)
        .map_err(|_| "Use a path inside the repository, like src/net.".to_owned())
}

impl Rocket {
    pub(crate) fn poll_history_search(&mut self, ctx: &Context) {
        let mut refresh_diffs = Vec::new();
        for tab in &mut self.tabs {
            if let Some(result) = tab
                .search_state
                .task
                .as_mut()
                .and_then(ReadTask::try_finish)
            {
                tab.search_state.task = None;
                let state = &mut tab.search_state;
                match result {
                    Ok(page) => {
                        state.commits.extend(page.commits);
                        state.scanned += page.scanned;
                        state.next = page.next;
                        state.complete = state.next.is_none();
                        if !state.complete && state.commits.len() < state.target {
                            state.due = Some(Instant::now());
                        }
                        if tab.selected_commit.is_none()
                            && let Some(first) = state.commits.first()
                        {
                            tab.selected_commit = Some(first.id.clone());
                            if tab.view == View::Commits {
                                refresh_diffs.push(tab.id);
                            }
                        }
                    }
                    Err(error) => {
                        state.bad_pattern = rocket_git::pattern_error(&error).is_some();
                        state.error = Some(match &error {
                            Error::InvalidInput(message) => message.clone(),
                            _ => user_error(&error),
                        });
                        state.paused = true;
                    }
                }
            }
        }
        for id in refresh_diffs {
            self.current_diff(id);
        }
        if self.quitting || self.write.is_some() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.loading() || tab.search_state.task.is_some() {
            return;
        }
        let Some(due) = tab.search_state.due else {
            return;
        };
        if Instant::now() < due {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
            return;
        }
        let Some(snapshot) = &tab.snapshot else {
            return;
        };
        let Some(history) = &snapshot.history else {
            tab.search_state.due = None;
            tab.search_state.error =
                Some("History is unavailable; refresh or choose another branch.".into());
            return;
        };
        let state = &mut tab.search_state;
        let start = state.next.clone().map_or_else(
            || HistorySearchStart::Query {
                start: history.start(),
                query: state.query.clone(),
                kind: state.kind,
                content: state.content.clone(),
            },
            HistorySearchStart::Continue,
        );
        let repository = snapshot.repository.clone();
        state.due = None;
        match ReadTask::spawn(move |cancel| backend.search_history(&repository, start, &cancel)) {
            Ok(task) => state.task = Some(task),
            Err(error) => {
                state.error = Some(user_error(&error));
                state.paused = true;
            }
        }
    }
}

/// The commit search through the app: each search kind against a real repository.
#[cfg(all(test, windows))]
mod flow_tests {
    use crate::{
        app::{Intent, Rocket},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::HistorySearchKind;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn settle(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            app.poll(ctx);
            if !app.busy() && !app.tabs[0].search_state.scanning() {
                return;
            }
            assert!(Instant::now() < deadline, "search did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn search(
        ctx: &Context,
        app: &mut Rocket,
        query: &str,
        kind: HistorySearchKind,
    ) -> Vec<String> {
        app.tabs[0].search = query.into();
        app.tabs[0].search_kind = kind;
        app.dispatch(Intent::Search);
        settle(ctx, app);
        let state = &app.tabs[0].search_state;
        assert!(state.complete && state.error.is_none(), "{:?}", state.error);
        assert_eq!(state.kind, kind);
        state
            .commits
            .iter()
            .map(|c| c.subject_lossy().into_owned())
            .collect()
    }

    #[test]
    fn commit_search_kinds_match_messages_paths_and_changed_text() {
        common::isolated(
            "ROCKET_HISTORY_SEARCH_UI_CASE",
            "history::flow_tests::commit_search_kinds_match_messages_paths_and_changed_text",
            || {
                let f = common::Fixture::new();
                f.write("src/Main.rs", "fn main() {}\n");
                f.commit("add entry point");
                f.write("README 文.md", "Needle text\n");
                f.commit("docs: readme");
                f.write("src/Main.rs", "fn main() { needle(); }\n");
                f.commit("call it");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                settle(&ctx, &mut app);
                app.auto_refresh = false;
                assert_eq!(
                    search(&ctx, &mut app, "main.rs", HistorySearchKind::Path),
                    ["call it", "add entry point"]
                );
                assert_eq!(
                    search(&ctx, &mut app, "readme 文", HistorySearchKind::Path),
                    ["docs: readme"]
                );
                assert_eq!(
                    search(&ctx, &mut app, "NEEDLE", HistorySearchKind::Content),
                    ["call it", "docs: readme"]
                );
                assert_eq!(
                    search(&ctx, &mut app, "readme", HistorySearchKind::Text),
                    ["docs: readme"]
                );
                assert!(search(&ctx, &mut app, "needle", HistorySearchKind::Text).is_empty());
                // Changed code: a path keeps changes under it, and exact text fills
                // the diff's Find so the selected commit shows where it changed.
                app.tabs[0].search_path = "src\\".into();
                assert_eq!(search(&ctx, &mut app, "needle", HistorySearchKind::Content), ["call it"]);
                assert_eq!(app.tabs[0].diff_tools.query, "needle");
                // An expression (git log -G) clears the Find it filled.
                app.tabs[0].search_regex = true;
                assert_eq!(search(&ctx, &mut app, "needle\\(", HistorySearchKind::Content), ["call it"]);
                assert!(app.tabs[0].diff_tools.query.is_empty());
                app.tabs[0].search = "needle(".into();
                app.dispatch(Intent::Search);
                settle(&ctx, &mut app);
                let state = &app.tabs[0].search_state;
                assert!(state.bad_pattern && state.commits.is_empty());
                assert!(state.error.as_deref().is_some_and(|e| e.starts_with("Not a valid regular expression: ")), "{:?}", state.error);
                // A path outside the repository never runs.
                app.tabs[0].search_regex = false;
                app.tabs[0].search_path = "../other".into();
                app.tabs[0].search = "needle".into();
                app.dispatch(Intent::Search);
                settle(&ctx, &mut app);
                assert_eq!(app.tabs[0].search_state.error.as_deref(), Some("Use a path inside the repository, like src/net."));
            },
        );
    }
}
