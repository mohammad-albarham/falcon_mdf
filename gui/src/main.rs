//! `falcon` — a desktop viewer for MF4 measurement files, built on
//! `falcon_mdf`. This is the launcher; the viewer itself is the
//! `falcon_mdf_gui` library beside it, which is where its logic can be
//! reached by a test.

use std::path::PathBuf;

use falcon_mdf_gui::app;

/// The window icon: 64x64 raw RGBA, the same spike motif `assets/icon.png`
/// carries into bundles. Embedded raw so the runtime needs no PNG decoder.
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon.rgba");

fn main() -> eframe::Result<()> {
    // A path on the command line opens directly, same as dropping a file
    // onto the window.
    let initial_path = std::env::args().nth(1).map(PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData {
                rgba: ICON_RGBA.to_vec(),
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };

    eframe::run_native(
        "falcon",
        native_options,
        Box::new(|cc| Ok(Box::new(app::FalconApp::new(cc, initial_path)))),
    )
}
