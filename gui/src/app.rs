//! Top-level application state and layout.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::job::Job;
use crate::loader::{spawn_load, LoadResult};
use crate::model::{ChannelLoc, LoadedFile, PlottedChannel};
use crate::panels::channel_list::ChannelBrowser;
use crate::panels::metadata;
use crate::panels::plot::PlotPanel;
use crate::recent::RecentFiles;

enum LoadState {
    Idle,
    Loading {
        path: PathBuf,
        rx: Receiver<LoadResult>,
    },
    Loaded(LoadedFile),
    /// A file that failed to open. Holds the full `Mf4Error` text, not a
    /// generic "failed to open" — several corpus files exercise unusual
    /// paths, and that text is the point of showing it.
    Failed {
        path: PathBuf,
        message: String,
    },
}

pub struct FalconApp {
    state: LoadState,
    recent: RecentFiles,
    browser: ChannelBrowser,
    selected: Option<ChannelLoc>,
    /// The channels the user has asked the plot panel to draw, in
    /// insertion order. Lives here rather than in `PlotPanel` because the
    /// channel list owns the interaction that adds and removes entries
    /// (checkbox, color swatch), while the plot panel only reads them.
    plotted: Vec<PlottedChannel>,
    plot: PlotPanel,
    /// Outcome of the last save/export action, shown in the metadata panel
    /// until the next one. A failed save must leave text somewhere, not a
    /// closed dialog and a silence.
    notice: Option<String>,
    /// Attachment saves run on a worker thread — embedded attachments are
    /// decompressed byte for byte, which on a large blob would freeze the
    /// frame loop. The message lands in `notice` once the worker finishes.
    attachment_job: Option<Job>,
    /// Last title sent to the window, so the viewport command is only re-sent
    /// when the file changes, not every frame.
    window_title: String,
}

impl FalconApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        let recent = RecentFiles::load(cc.storage);
        let mut app = Self {
            state: LoadState::Idle,
            recent,
            browser: ChannelBrowser::new(),
            selected: None,
            plotted: Vec::new(),
            plot: PlotPanel::new(),
            notice: None,
            attachment_job: None,
            window_title: "falcon".to_string(),
        };
        if let Some(path) = initial_path {
            app.start_load(path, &cc.egui_ctx);
        }
        app
    }

    fn start_load(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.selected = None;
        self.browser.reset();
        // A `ChannelLoc` is just indices, so a stale plot from the previous
        // file could otherwise look "already loaded" for a channel at the
        // same (dg, cg, ch) position in the new one. Clearing both sides of
        // the seam keeps the new file starting from an empty plot.
        self.plotted.clear();
        self.plot = PlotPanel::new();
        self.notice = None;
        // A save started for the previous file belongs to it; its result
        // would arrive into the new file's notice line otherwise. The worker
        // still finishes and writes what it was asked to.
        self.attachment_job = None;
        let rx = spawn_load(path.clone(), ctx.clone());
        self.state = LoadState::Loading { path, rx };
    }

    fn poll_load(&mut self) {
        let LoadState::Loading { path, rx } = &self.state else {
            return;
        };
        match rx.try_recv() {
            Ok(LoadResult::Ok(loaded)) => {
                self.recent.push(&loaded.path);
                self.state = LoadState::Loaded(loaded);
            }
            Ok(LoadResult::Err { path, message }) => {
                self.state = LoadState::Failed { path, message };
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Name the file the failure panel is about: an empty path
                // there reads like a bug in the panel, not in the load.
                let path = path.clone();
                self.state = LoadState::Failed {
                    path,
                    message: "loader thread ended without a result".to_string(),
                };
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.into_iter().find_map(|f| f.path) {
            self.start_load(path, ctx);
        }
    }

    /// The window title names the open file, so a screenshot or a window
    /// switcher says which measurement this is — a viewer whose title is
    /// always the app name makes every window look like every other.
    fn update_title(&mut self, ctx: &egui::Context) {
        let desired = match &self.state {
            LoadState::Idle => "falcon".to_string(),
            LoadState::Loading { .. } => "falcon — opening\u{2026}".to_string(),
            LoadState::Failed { .. } => "falcon — open failed".to_string(),
            LoadState::Loaded(loaded) => {
                let name = loaded
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| loaded.path.display().to_string());
                format!("falcon — {name}")
            }
        };
        if desired != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(desired.clone()));
            self.window_title = desired;
        }
    }

    /// Top bar: open/recent-files controls and the loading spinner. Takes
    /// the outer `ui` and claims a strip off its top, same as every other
    /// panel below claims a strip of whatever `ui` is left after this one.
    fn top_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Panel::top("top_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open File\u{2026}").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("MF4", &["mf4", "MF4"])
                        .pick_file()
                    {
                        self.start_load(path, ctx);
                    }
                }

                ui.menu_button("Recent Files", |ui| {
                    if self.recent.paths().is_empty() {
                        ui.label("(none)");
                    }
                    for path in self.recent.paths().to_vec() {
                        if ui.button(path.display().to_string()).clicked() {
                            self.start_load(path, ctx);
                            ui.close();
                        }
                    }
                });

                if let LoadState::Loading { path, .. } = &self.state {
                    ui.separator();
                    ui.spinner();
                    ui.label(format!("Opening {}\u{2026}", path.display()));
                }
            });
        });
    }
}

impl eframe::App for FalconApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.poll_load();
        self.handle_dropped_files(&ctx);
        self.update_title(&ctx);
        self.top_panel(ui, &ctx);

        match &self.state {
            LoadState::Idle => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("No file open");
                        ui.label("Use \u{201c}Open File\u{2026}\u{201d}, drop an MF4 file onto this window, or pick a recent file.");
                    });
                });
            }
            LoadState::Loading { .. } => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.spinner();
                        ui.label("Opening file\u{2026}");
                    });
                });
            }
            LoadState::Failed { path, message } => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("Failed to open file");
                        ui.label(path.display().to_string());
                        ui.add_space(10.0);
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), message);
                    });
                });
            }
            LoadState::Loaded(loaded) => {
                // A finished attachment save becomes the notice line; the job
                // itself is dropped once its message has landed.
                if let Some(job) = &self.attachment_job {
                    if let Some(message) = job.poll() {
                        self.notice = Some(message);
                        self.attachment_job = None;
                    }
                }

                egui::Panel::right("metadata_panel")
                    .resizable(true)
                    .default_size(280.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.heading("File Info");
                            metadata::show_file_metadata(
                                ui,
                                loaded,
                                &mut self.notice,
                                &mut self.plotted,
                                &mut self.attachment_job,
                            );
                        });
                    });

                egui::Panel::left("channel_panel")
                    .resizable(true)
                    .default_size(360.0)
                    .show(ui, |ui| {
                        ui.heading("Channels");
                        self.browser
                            .show(ui, loaded, &mut self.plotted, &mut self.selected);
                    });

                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(loc) = self.selected {
                        egui::CollapsingHeader::new("Channel Detail")
                            .default_open(false)
                            .show(ui, |ui| metadata::show_channel_detail(ui, loaded, loc));
                    }
                    // The plot panel owns its empty-state text, so it is
                    // shown unconditionally — with no plotted channels it
                    // explains how to add one.
                    self.plot.show(ui, loaded, &self.plotted);
                });
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.save(storage);
    }
}
