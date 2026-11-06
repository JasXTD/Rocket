#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use eframe::egui;
use rocket_ui::app::Rocket;
use std::path::PathBuf;

fn main() -> eframe::Result {
    if let Err(error) = rocket_git::ensure_unprivileged_process() {
        // No profile, repository, updater plan or link handler is initialized.
        let message = error.to_string();
        eprintln!("{message}");
        eframe::run_native("Rocket cannot start", eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 180.0]).with_resizable(false),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        }, Box::new(move |_| Ok(Box::new(StartupRefusal(message)))))?;
        return Err(eframe::Error::AppCreation(Box::new(error)));
    }
    // Capture errors remain visible in Help > Crash Reports; startup stays usable.
    let _ = rocket_git::crash_reports::install(rocket_git::crash_reports::default_directory(),rocket_git::crash_reports::Component::Desktop);
    // The desktop app alone follows the system's language; Preferences can pick another.
    let languages = rocket_git::system_languages();
    rocket_ui::i18n::start(&languages.display, languages.region.as_deref());
    let mut args:Vec<_>=std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a|a=="--rocket-apply-update") {
        let result=if args.len()==2 {rocket_ui::app_updates::install::apply(&PathBuf::from(&args[1]))}else{Err(rocket_git::Error::InvalidInput("Expected one local update plan.".into()))};
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}
        return Ok(());
    }
    let update_result=if args.len()>=2&&args[args.len()-2]=="--rocket-update-result" {
        let path=PathBuf::from(args.pop().unwrap());args.pop();Some(path)
    }else{None};
    let launch=match rocket_ui::reviews::launch_args(&args) {
        Ok(value)=>value,
        Err(error)=>{eprintln!("{error}");std::process::exit(2);}
    };
    let paths: Vec<PathBuf> = launch.as_ref().map_or_else(||args.into_iter().map(PathBuf::from).collect(),|(path,_)|vec![path.clone()]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rocket")
            .with_icon(rocket_ui::app_icon::icon())
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([780.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rocket",
        options,
        Box::new(move |cc| {let mut app=Rocket::new(&cc.egui_ctx,paths);app.updates.result_directory=update_result;app.pending_review=launch.map(|(path,request)|(std::fs::canonicalize(&path).unwrap_or(path),request));Ok(Box::new(app))}),
    )
}

struct StartupRefusal(String);
impl eframe::App for StartupRefusal {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("Open Rocket normally");
            ui.add_space(12.0);
            ui.label(&self.0);
            ui.add_space(12.0);
            if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}
