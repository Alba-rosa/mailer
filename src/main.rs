#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autoconfig;
mod mail;
mod secret;
mod store;
mod text;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Mailer")
            .with_app_id("mailer")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("mailer", options, Box::new(|cc| Ok(Box::new(app::MailerApp::new(cc)))))
}
