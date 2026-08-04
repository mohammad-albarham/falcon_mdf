//! `falcon` — a desktop viewer for MF4 measurement files, built on
//! `falcon_mdf`. Opens files, browses their channels (G1), and plots the
//! selected one against its master with decimation (G2).

mod app;
mod loader;
mod model;
mod panels;
mod recent;
mod signal_loader;

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
