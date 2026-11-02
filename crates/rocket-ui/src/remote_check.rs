//! Test connection in the Add remote, Edit remote URL and Clone dialogs: a
//! cancellable, time-limited `git ls-remote` that never asks for credentials.
use crate::{
    app::{Dialog, DialogKind, UiControl},
    backend::{Backend, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

pub struct ConnectionTest {
    task: Option<ReadTask<RemoteCheck>>,
    started: Instant,
    /// The URL or path tested; an edited field drops the old result.
    pub target: String,
    pub result: Option<std::result::Result<RemoteCheck, Failure>>,
}

/// A failed test in words, with a stable code for diagnostics and automation.
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

/// What the dialog would test now: the URL or path field, or for a push URL
/// that falls back to the fetch URL, that fetch URL.
pub fn target(dialog: &Dialog) -> Option<String> {
    let text = match &dialog.kind {
        DialogKind::AddRemote => dialog.second.trim().to_owned(),
        DialogKind::EditRemote(remote, kind) if *kind == RemoteUrlKind::Push && dialog.flag => {
            remote.urls.first().map(|u| display(u))?
        }
        DialogKind::EditRemote(..) | DialogKind::Clone => dialog.first.trim().to_owned(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

impl ConnectionTest {
    /// Test `target` for the repository at `root`, or as a clone source.
    fn start(backend: &Backend, root: Option<&Path>, target: String) -> Self {
        let backend = backend.clone();
        let tested = target.clone();
        let task = match root {
            Some(root) => {
                let root = root.to_path_buf();
                ReadTask::spawn(move |cancel| {
                    backend.check_remote(&root, target.as_bytes(), &cancel)
                })
            }
            None => {
                // The Clone dialog's own rule for URLs and local folders.
                let source = if target.starts_with("https://") || target.starts_with("ssh://") {
                    CloneSource::Url(target)
                } else {
                    CloneSource::Local(PathBuf::from(target))
                };
                ReadTask::spawn(move |cancel| backend.check_clone_source(&source, &cancel))
            }
        };
        let mut test = Self {
            task: None,
            started: Instant::now(),
            target: tested,
            result: None,
        };
        match task {
            Ok(task) => test.task = Some(task),
            Err(error) => test.result = Some(Err(failure(&error))),
        }
        test
    }

    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    pub(crate) fn poll(&mut self) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            self.result = Some(result.map_err(|e| failure(&e)));
        }
    }
}

fn failure(error: &Error) -> Failure {
    let (code, message) = match error {
        Error::Cancelled => ("cancelled", "Test stopped. Nothing was changed.".to_owned()),
        Error::TimedOut => (
            "timed_out",
            format!(
                "No answer within {} seconds. Check the URL, network, VPN and proxy; the server may be unreachable.",
                REMOTE_CHECK_LIMIT.as_secs()
            ),
        ),
        Error::InvalidInput(message) => ("invalid_input", message.clone()),
        _ => match (error.fetch_failure(), error.clone_failure()) {
            (Some(f), _) => (
                f.code(),
                format!("Could not connect: {}\nNext: {}", f.summary(), f.guidance()),
            ),
            (_, Some(f)) => (
                f.code(),
                format!(
                    "Could not use this source: {}\nNext: {}",
                    f.summary(),
                    f.guidance()
                ),
            ),
            _ => ("failed", user_error(error)),
        },
    };
    Failure { code, message }
}

/// Test connection for the end of a field row, or Stop while a test runs; the
/// progress and result go below the row with [`inline_status`].
pub(crate) fn inline_button(
    ui: &mut Ui,
    test: &mut Option<ConnectionTest>,
    target: Option<String>,
    backend: Option<&Backend>,
    root: Option<&Path>,
    widgets: &mut HashMap<String, UiControl>,
) {
    refresh(test, target.as_deref());
    match test.as_ref().filter(|t| t.running()) {
        Some(t) => stop_button(ui, t, widgets),
        None => start_button(ui, test, target, backend, root, widgets),
    }
}

/// The running test's seconds, then the last result, for [`inline_button`].
pub(crate) fn inline_status(ui: &mut Ui, test: &Option<ConnectionTest>) {
    if let Some(t) = test.as_ref().filter(|t| t.running()) {
        ui.horizontal(|ui| running(ui, t));
    }
    result(ui, test);
}

/// Take a finished test's result; drop one for a field that has changed since.
fn refresh(test: &mut Option<ConnectionTest>, target: Option<&str>) {
    if let Some(t) = test.as_mut() {
        t.poll();
        if !t.running() && target != Some(t.target.as_str()) {
            *test = None; // The field changed: the result no longer applies.
        }
    }
}

fn running(ui: &mut Ui, t: &ConnectionTest) {
    ui.spinner();
    ui.label(
        RichText::new(format!(
            "Testing connection… {} s",
            t.started.elapsed().as_secs()
        ))
        .color(MUTED()),
    );
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(250));
}

fn stop_button(ui: &mut Ui, t: &ConnectionTest, widgets: &mut HashMap<String, UiControl>) {
    let stop = ui.button("Stop");
    pages::control(widgets, "dialog-test-connection-stop", &stop, ui.clip_rect());
    if stop.clicked()
        && let Some(task) = &t.task
    {
        task.cancel();
    }
}

fn start_button(
    ui: &mut Ui,
    test: &mut Option<ConnectionTest>,
    target: Option<String>,
    backend: Option<&Backend>,
    root: Option<&Path>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let enabled = target.is_some() && backend.is_some();
    let button = ui
        .add_enabled(enabled, Button::new("Test connection"))
        .on_hover_text(format!(
            "Lists the remote's branches with git ls-remote, without asking for credentials. Stops after {} seconds; nothing is saved or downloaded.",
            REMOTE_CHECK_LIMIT.as_secs()
        ));
    pages::control(widgets, "dialog-test-connection", &button, ui.clip_rect());
    if button.clicked()
        && let (Some(backend), Some(target)) = (backend, target)
    {
        *test = Some(ConnectionTest::start(backend, root, target));
    }
}

/// The last result as a callout.
fn result(ui: &mut Ui, test: &Option<ConnectionTest>) {
    match test.as_ref().and_then(|t| t.result.as_ref()) {
        Some(Ok(check)) => {
            let branches = match check.branches {
                0 => "the repository has no branches yet".to_owned(),
                1 => "1 branch".to_owned(),
                n => format!("{n} branches"),
            };
            let default = check
                .default_branch
                .as_ref()
                .filter(|_| check.branches > 0)
                .map(|b| format!("; default branch {}", display(b)))
                .unwrap_or_default();
            pages::callout(
                ui,
                GREEN(),
                &format!(
                    "Connected in {:.1} s: {branches}{default}.",
                    check.elapsed.as_secs_f32()
                ),
            );
        }
        Some(Err(failure)) => pages::callout(
            ui,
            if failure.code == "cancelled" {
                AMBER()
            } else {
                RED()
            },
            &failure.message,
        ),
        None => {}
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

    /// Test connection in Add remote: a local bare repository connects and lists
    /// its branch; editing the field drops that result; a folder that is not a
    /// repository fails with a code. Nothing is added or fetched.
    #[test]
    fn add_remote_tests_the_typed_url_without_saving_it() {
        common::isolated(
            "ROCKET_REMOTE_CHECK_UI_CASE",
            "remote_check::tests::add_remote_tests_the_typed_url_without_saving_it",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "a");
                f.commit("first");
                let bare = f.root.join("origin.git");
                f.run(&["clone", "--bare", "--quiet", ".", bare.to_str().unwrap()]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::AddRemote));
                frame(&ctx, &mut app, vec![]);
                app.dialog.as_mut().unwrap().second = bare.to_string_lossy().into_owned();
                click(&ctx, &mut app, "dialog-test-connection");
                wait(&ctx, &mut app);
                let test = app.dialog.as_ref().unwrap().connection.as_ref().unwrap();
                let check = test.result.as_ref().unwrap().as_ref().ok().unwrap();
                assert_eq!(check.branches, 1);
                assert_eq!(check.default_branch.as_deref(), Some(&b"main"[..]));

                let folder = f.root.join("not a repository");
                std::fs::create_dir(&folder).unwrap();
                app.dialog.as_mut().unwrap().second = folder.to_string_lossy().into_owned();
                frame(&ctx, &mut app, vec![]);
                assert!(app.dialog.as_ref().unwrap().connection.is_none());
                click(&ctx, &mut app, "dialog-test-connection");
                wait(&ctx, &mut app);
                let test = app.dialog.as_ref().unwrap().connection.as_ref().unwrap();
                let failure = test.result.as_ref().unwrap().as_ref().err().unwrap();
                assert!(!["cancelled", "timed_out"].contains(&failure.code));
                assert!(f.run(&["remote"]).is_empty());
            },
        );
    }
}
