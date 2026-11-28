//! Gimbly — an unofficial, independent control app for OBSBOT cameras.
//! Not made, approved or supported by OBSBOT / Remo Tech.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod branding;
mod camera;
mod gimbal;
mod hotkeys;
mod joystick;
mod os;
mod preview_tracking;
mod preview_view;
mod preview_window;
mod probe;
mod record;
#[cfg(target_os = "linux")]
mod serial_access;
mod settings;
mod settings_ui;
mod sleep_pictures;
mod snapshot;
mod theme;
mod vcam;
mod widgets;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--probe") {
        os::attach_parent_console();
        std::process::exit(probe::run());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Gimbly")
            .with_app_id(crate::branding::APP_ID)
            .with_icon(branding::window_icon())
            .with_inner_size([theme::PANEL_WIDTH, 840.0])
            .with_min_inner_size([theme::PANEL_WIDTH, 480.0])
            .with_max_inner_size([theme::PANEL_WIDTH, 4096.0])
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Gimbly",
        options,
        Box::new(|cc| Ok(Box::new(app::GimblyApp::new(cc)))),
    )
}
