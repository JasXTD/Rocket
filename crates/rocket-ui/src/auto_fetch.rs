//! Automatic fetch: every remote of every open repository, every few minutes, when
//! turned on in Preferences. It never prompts, prunes or merges, and steps aside
//! when a write starts. A failure is shown quietly and retried at the next interval.
use crate::{app::*, backend::user_error, pages, style::*};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// One repository's automatic fetch.
#[derive(Default)]
pub struct TabFetch {
    next: Option<Instant>,
    task: Option<ReadTask<QuietFetchReport>>,
    /// When the last fetch finished, and each remote that failed with the reason.
    pub last: Option<(Instant, Vec<String>)>,
}
impl TabFetch {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// Time until the next attempt.
    pub fn due_in(&self) -> Option<Duration> {
        self.next
            .map(|next| next.saturating_duration_since(Instant::now()))
    }
}

#[derive(Default)]
pub struct AutoFetch {
    /// Minutes between fetches; 0 is off. Saved with the preferences.
    pub minutes: u16,
    /// A shorter interval, set only by the isolated MCP fixture host.
    pub test_interval: Option<Duration>,
    /// Set by hosts that must not contact remotes (the MCP host without its marker).
    pub blocked: bool,
    pub tabs: HashMap<u64, TabFetch>,
}
impl AutoFetch {
    fn interval(&self) -> Option<Duration> {
        (self.minutes != 0 && !self.blocked).then(|| {
            self.test_interval
                .unwrap_or(Duration::from_secs(u64::from(self.minutes) * 60))
        })
    }
    /// Stop a running fetch of this repository, for example because a write is
    /// starting; the next attempt is a full interval later.
    pub fn stand_aside(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab)
            && state.task.take().is_some()
        {
            state.next = None;
        }
    }
}

/// Preferences: the switch and, while on, the interval. Off by default.
pub(crate) fn preferences(
    ui: &mut Ui,
    minutes: &mut u16,
    widgets: &mut HashMap<String, UiControl>,
) {
    let mut on = *minutes != 0;
    let r = crate::preferences::bar_switch_row(
        ui,
        widgets,
        "pref-auto-fetch",
        &mut on,
        "Fetch remotes automatically",
        "All remotes of open repositories",
        "",
    );
    if r.changed() {
        *minutes = if on { 15 } else { 0 };
    }
    if *minutes != 0 {
        Frame::new()
            .inner_margin(Margin {
                left: 14,
                right: 14,
                top: 0,
                bottom: 12,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(RichText::new("Every").color(MUTED()));
                    let choices = &crate::persistence::AUTO_FETCH_MINUTES[1..];
                    let labels: Vec<String> = choices.iter().map(|c| format!("{c} min")).collect();
                    let items: Vec<(&str, bool)> = choices
                        .iter()
                        .zip(&labels)
                        .map(|(c, label)| (label.as_str(), *minutes == *c))
                        .collect();
                    let ids: Vec<String> = choices.iter().map(|c| format!("pref-auto-fetch-{c}")).collect();
                    if let Some(i) = crate::preferences::choices(ui, widgets, &items, &ids) {
                        *minutes = choices[i];
                    }
                });
            });
    }
}

impl Rocket {
    pub(crate) fn poll_auto_fetch(&mut self) {
        let Some(interval) = self.auto_fetch.interval() else {
            // Dropping a task cancels its fetch.
            self.auto_fetch.tabs.clear();
            return;
        };
        let now = Instant::now();
        let open: Vec<(u64, std::path::PathBuf, bool)> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.path.clone(), t.loading()))
            .collect();
        self.auto_fetch
            .tabs
            .retain(|id, _| open.iter().any(|(t, ..)| t == id));
        let idle = self.write.is_none() && !self.quitting && !self.benchmark.running();
        let mut changed = Vec::new();
        for (id, path, loading) in open {
            let state = self.auto_fetch.tabs.entry(id).or_default();
            if let Some(result) = state.task.as_mut().and_then(ReadTask::try_finish) {
                state.task = None;
                state.next = Some(now + interval);
                let failures = match result {
                    Ok(report) => {
                        if report.changed() {
                            changed.push(id);
                        }
                        report
                            .results
                            .iter()
                            .filter_map(|(name, r)| {
                                r.as_ref()
                                    .err()
                                    .map(|e| format!("{}: {}", display(name), user_error(e)))
                            })
                            .collect()
                    }
                    Err(e) => vec![user_error(&e)],
                };
                state.last = Some((now, failures));
                continue;
            }
            let next = *state.next.get_or_insert(now + interval);
            if state.task.is_none() && now >= next && idle && !loading {
                let Some(backend) = self.backend.clone() else {
                    continue;
                };
                let tags = self.general.fetch_tags;
                match ReadTask::spawn(move |cancel| backend.fetch_all_quietly(&path, tags, &cancel)) {
                    Ok(task) => state.task = Some(task),
                    Err(e) => {
                        state.next = Some(now + interval);
                        state.last = Some((now, vec![user_error(&e)]));
                    }
                }
            }
        }
        for id in changed {
            if self.write.is_none() {
                self.refresh(id);
            }
        }
    }

    /// Status bar: a quiet note while fetching automatically or after a failure.
    pub(crate) fn auto_fetch_status(&mut self, ui: &mut Ui) {
        let Some(id) = self.tab().map(|t| t.id) else {
            return;
        };
        let Some(state) = self.auto_fetch.tabs.get(&id) else {
            return;
        };
        if state.running() {
            ui.label(RichText::new("Fetching…").size(11.5).color(FAINT()))
                .on_hover_text(
                    "Automatic fetch of every remote. It never prompts, prunes or merges.",
                );
            let r = ui.add(Button::new(RichText::new("Stop").size(11.5)).frame(false));
            pages::control(
                &mut self.frame_widgets,
                "auto-fetch-stop",
                &r,
                ui.clip_rect(),
            );
            if r.clicked() {
                self.auto_fetch.stand_aside(id);
            }
        } else if let Some((_, failures)) = &state.last
            && !failures.is_empty()
        {
            let r = ui
                .label(
                    RichText::new("Automatic fetch failed")
                        .size(11.5)
                        .color(AMBER()),
                )
                .on_hover_text(format!(
                    "{}\nRetried at the next interval. Automatic fetch never asks for credentials; fetch manually to sign in.",
                    failures.join("\n")
                ));
            pages::control(
                &mut self.frame_widgets,
                "auto-fetch-failed",
                &r,
                ui.clip_rect(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_fetch_steps_aside_for_a_write() {
        let task = ReadTask::spawn(|cancel: CancellationToken| -> Result<QuietFetchReport> {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(Error::Cancelled)
        })
        .unwrap();
        let mut fetch = AutoFetch {
            minutes: 5,
            ..Default::default()
        };
        fetch.tabs.insert(
            1,
            TabFetch {
                next: Some(Instant::now()),
                task: Some(task),
                last: None,
            },
        );
        assert!(fetch.tabs[&1].running());
        fetch.stand_aside(1);
        assert!(!fetch.tabs[&1].running() && fetch.tabs[&1].next.is_none());
        assert_eq!(fetch.interval(), Some(Duration::from_secs(300)));
        fetch.blocked = true;
        assert_eq!(fetch.interval(), None);
    }
}

#[cfg(test)]
mod flow_tests {
    use crate::{batch::flow_tests::open, test_common as common};
    use eframe::egui::Context;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn automatic_fetch_runs_on_its_interval_and_refreshes_branches() {
        common::isolated(
            "ROCKET_AUTO_FETCH_UI_CASE",
            "auto_fetch::flow_tests::automatic_fetch_runs_on_its_interval_and_refreshes_branches",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let remote = f.root.join("origin.git");
                f.run(&["init", "--quiet", "--bare", remote.to_str().unwrap()]);
                f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
                f.run(&["push", "--quiet", "origin", "main", "main:arrived"]);
                f.run(&["update-ref", "-d", "refs/remotes/origin/arrived"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let id = app.tabs[0].id;
                app.auto_fetch.test_interval = Some(Duration::from_millis(200));
                app.auto_fetch.minutes = 5;
                let deadline = Instant::now() + Duration::from_secs(30);
                let tracked = |app: &crate::app::Rocket| {
                    app.tabs[0].snapshot.as_ref().is_some_and(|s| {
                        s.branches
                            .iter()
                            .any(|b| b.reference == b"refs/remotes/origin/arrived")
                    })
                };
                while !(app
                    .auto_fetch
                    .tabs
                    .get(&id)
                    .is_some_and(|t| t.last.is_some())
                    && tracked(&app))
                {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "no automatic fetch");
                    thread::sleep(Duration::from_millis(20));
                }
                let (_, failures) = app.auto_fetch.tabs[&id].last.clone().unwrap();
                assert!(failures.is_empty(), "{failures:?}");
                // Turned off: nothing is scheduled or running.
                app.auto_fetch.minutes = 0;
                app.poll(&ctx);
                assert!(app.auto_fetch.tabs.is_empty());
            },
        );
    }
}
