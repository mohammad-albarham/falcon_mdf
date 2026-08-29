//! `falcon` — a desktop viewer for MF4 measurement files, built on
//! `falcon_mdf`. This is the launcher; the viewer itself is the
//! `falcon_mdf_gui` library beside it, which is where its logic can be
//! reached by a test.

use std::process::ExitCode;

use falcon_mdf_gui::app;
use falcon_mdf_gui::cli::{self, Launch};

/// The window icon: 64x64 raw RGBA, the same spike motif `assets/icon.png`
/// carries into bundles. Embedded raw so the runtime needs no PNG decoder.
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon.rgba");

fn main() -> ExitCode {
    // A path on the command line opens directly, same as dropping a file onto
    // the window. `--help` and `--version` answer and exit without ever
    // reaching a display, so they work over SSH and in a package's smoke test.
    let initial_path = match cli::parse(std::env::args().skip(1)) {
        Launch::Window(path) => path,
        Launch::Help => {
            println!("{}", cli::HELP);
            return ExitCode::SUCCESS;
        }
        Launch::Version => {
            println!("{}", cli::VERSION);
            return ExitCode::SUCCESS;
        }
        Launch::Usage(message) => {
            eprintln!("falcon: {message}");
            // 2, not 1: a usage error is not a failed read, and scripts that
            // wrap the viewer can tell them apart.
            return ExitCode::from(2);
        }
    };

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

    match eframe::run_native(
        "falcon",
        native_options,
        Box::new(|cc| Ok(Box::new(app::FalconApp::new(cc, initial_path)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        // A window that cannot be created is the one failure with nowhere on
        // screen to report itself, so it goes to stderr — the machine with no
        // display, the broken GPU driver, the missing Wayland socket.
        Err(e) => {
            eprintln!("falcon: could not open a window: {e}");
            ExitCode::FAILURE
        }
    }
}
