use super::Rocket;
use crate::{folder_browser::PickTarget, pages, preferences, style::*};
use eframe::egui::*;

impl Rocket {
    pub(super) fn dependency_setup_page(
        &mut self,
        ui: &mut Ui,
        owner: Option<u64>,
    ) -> (bool, bool) {
        let running = self.dependencies.running();
        let ready = self.dependencies.setup.ready;
        let editable = self.dependencies.editable;
        let loaded = self.dependencies.setup.loaded;
        let data_ready = owner
            .and_then(|id| self.dependencies.repositories.get(&id))
            .is_some_and(|state| state.cached.is_some() && state.error.is_none());
        let mut choose = false;
        let mut reload = false;
        let mut stop = false;
        let mut scan = false;
        let mut update = false;
        preferences::card_frame(ui, |ui| {
            preferences::card_note(ui, |ui| {
                ui.label(RichText::new("Scanner & advisory data").font(semibold(14.0)));
                ui.label(
                    RichText::new("Set up once for this Rocket profile.")
                        .size(12.0)
                        .color(MUTED()),
                );
            });
            preferences::divider(ui);
            Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("1 · Choose OSV-Scanner").strong());
            if self.dependencies.setup.running() {
                ui.label(RichText::new("Checking the scanner…").color(BLUE()));
            } else if ready {
                ui.label(
                    RichText::new(format!("OSV-Scanner {} verified", rocket_git::OSV_VERSION))
                        .color(GREEN()),
                );
            } else {
                ui.label(format!(
                    "OSV-Scanner {} · {} · verified before use",
                    rocket_git::OSV_VERSION,
                    rocket_git::osv_platform().unwrap_or("Windows x64 or Linux x64")
                ));
            }
            ui.horizontal_wrapped(|ui| {
                let r = ui.add_enabled(
                    editable && loaded && !running,
                    Button::new(if ready {
                        "Change scanner…"
                    } else {
                        "Choose scanner…"
                    }),
                );
                pages::control(
                    &mut self.frame_widgets,
                    "dependency-scanner-choose",
                    &r,
                    ui.clip_rect(),
                );
                choose = r.clicked();
                let r = ui.hyperlink_to(
                    "Get OSV-Scanner ↗",
                    format!(
                        "https://github.com/google/osv-scanner/releases/tag/v{}",
                        rocket_git::OSV_VERSION
                    ),
                );
                pages::control(
                    &mut self.frame_widgets,
                    "dependency-scanner-release",
                    &r,
                    ui.clip_rect(),
                );
                if self.dependencies.setup.error.is_some() {
                    let r = ui.add_enabled(!running, Button::new("Reload setup"));
                    pages::control(
                        &mut self.frame_widgets,
                        "dependency-scanner-reload",
                        &r,
                        ui.clip_rect(),
                    );
                    reload = r.clicked();
                }
            });
            if let Some(error) = &self.dependencies.setup.error {
                pages::callout(ui, AMBER(), error);
            }
            if ready {
                let response = ui.collapsing("Scanner location", |ui| {
                    let (program, _, _) = self.dependency_paths();
                    ui.add(Label::new(program.display().to_string()).wrap());
                    ui.label(RichText::new("Rocket verifies the scanner checksum before every launch.").size(12.0).color(MUTED()));
                });
                pages::control(&mut self.frame_widgets, "dependency-scanner-location", &response.header_response, ui.clip_rect());
            }
            ui.separator();
            ui.label(RichText::new("2 · Download advisory data").strong());
            if owner.is_none() {
                ui.label(
                    "Open a repository to download the public databases its dependencies need.",
                );
            } else if data_ready {
                ui.label(
                    RichText::new("Local advisory data is ready for this repository.")
                        .color(GREEN()),
                );
            } else if !ready {
                ui.label("Choose a scanner first, then download advisory data.");
            } else {
                ui.label(
                    "Download the public databases needed by this repository to begin scanning.",
                );
            }
            ui.horizontal_wrapped(|ui| {
                let r = pages::primary(ui,
                    owner.is_some() && ready && !running && editable,
                    if data_ready {
                        "Update advisory data"
                    } else {
                        "Download advisory data"
                    },
                );
                pages::control(
                    &mut self.frame_widgets,
                    "dependencies-update",
                    &r,
                    ui.clip_rect(),
                );
                update = r.clicked();
                let r = ui.add_enabled(
                    owner.is_some() && ready && !running,
                    Button::new("Scan dependencies"),
                );
                pages::control(
                    &mut self.frame_widgets,
                    "dependencies-scan",
                    &r,
                    ui.clip_rect(),
                );
                scan = r.clicked();
                if running {
                    let r = ui.button("Stop");
                    pages::control(
                        &mut self.frame_widgets,
                        "dependencies-cancel",
                        &r,
                        ui.clip_rect(),
                    );
                    stop = r.clicked();
                    ui.spinner();
                    let label = if self.dependencies.job.as_ref().is_some_and(|j| j.update) {
                        "Downloading public advisory data…"
                    } else {
                        "Checking…"
                    };
                    ui.label(RichText::new(label).size(12.0).color(HINT()));
                }
            });
            ui.label(RichText::new("Downloads can be large. Repository files and package lists stay on this computer. Later scans run offline.").size(12.0).color(HINT()));
            });
            preferences::divider(ui);
            preferences::card_note(ui, |ui| {
                let response = ui.collapsing("Storage & coverage", |ui| {
                    let (_, cache, _) = self.dependency_paths();
                    ui.add(Label::new(format!("Local advisory data: {}", cache.display())).wrap());
                    ui.label("The scanner choice and advisory data belong to this Rocket profile. Scans run offline and do not execute project scripts.");
                    ui.label("Recognizes Rust, npm/pnpm/Yarn/Bun, Python, Go, NuGet, Gradle, Composer, Ruby, Dart, Elixir and Conan dependency files. Missing lockfiles and indirect versions may limit coverage. Generated dependency folders, symbolic links and junctions are skipped.");
                });
                pages::control(
                    &mut self.frame_widgets,
                    "dependency-storage-coverage",
                    &response.header_response,
                    ui.clip_rect(),
                );
            });
        });
        if choose {
            let current = self
                .dependencies
                .setup
                .program
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            self.browse(PickTarget::DependencyScanner, &current);
        }
        if reload {
            self.reload_dependency_setup();
        }
        if stop {
            self.dependencies.cancel();
        }
        (scan, update)
    }
}
