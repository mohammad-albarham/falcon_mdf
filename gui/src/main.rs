//! `falcon` — a desktop viewer for MF4 measurement files, built on
//! `falcon_mdf`. This binary is the shell (G1): opening files and browsing
//! their channels. G2 adds plotting on top of the panels defined here.

mod app;
mod loader;
mod model;
mod panels;
mod recent;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // A path on the command line opens directly, same as dropping a file
    // onto the window.
    let initial_path = std::env::args().nth(1).map(PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "falcon",
        native_options,
        Box::new(|cc| Ok(Box::new(app::FalconApp::new(cc, initial_path)))),
    )
}
