//! The batch tab: a queue of files, one operation, applied to all of them.
//!
//! The logic is in [`crate::batch`] and [`crate::batch_queue`], which need no
//! window; what is here is the queue list, the operation's controls, and the
//! per-file outcomes a run produces. The run itself happens on a worker
//! thread, so a batch over gigabyte files leaves the viewer responsive and
//! can be cancelled between files.

use std::path::PathBuf;

use crate::batch::{spawn, summarise, BatchOp, BatchRun};
use crate::batch_queue::BatchQueue;

/// Which operation the tab is set to, held apart from [`BatchOp`] so the
/// controls keep what was typed into them when the choice changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpChoice {
    Cut,
    Filter,
    Export,
}

impl OpChoice {
    const ALL: [OpChoice; 3] = [OpChoice::Cut, OpChoice::Filter, OpChoice::Export];

    fn label(self) -> &'static str {
        match self {
            OpChoice::Cut => "Cut to time range",
            OpChoice::Filter => "Keep channels",
            OpChoice::Export => "Export to CSV",
        }
    }
}

/// The batch tab's own state: what is queued, what to do, and what happened.
pub struct BatchPanel {
    choice: OpChoice,
    start: String,
    end: String,
    names: String,
    /// Where output goes. `None` writes beside each input.
    out_dir: Option<PathBuf>,
    run: Option<BatchRun>,
    /// The summary of the last finished run, kept after the run is dropped.
    last_summary: Option<String>,
    /// How many files the run in progress was started with, so progress is a
    /// fraction of the queue as it was, not as it is now.
    run_total: usize,
}

impl Default for BatchPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchPanel {
    pub fn new() -> Self {
        Self {
            choice: OpChoice::Cut,
            start: "0".to_string(),
            end: "10".to_string(),
            names: String::new(),
            out_dir: None,
            run: None,
            last_summary: None,
            run_total: 0,
        }
    }

    /// The operation the controls currently describe, or why they do not
    /// describe one.
    fn current_op(&self) -> Result<BatchOp, String> {
        let op = match self.choice {
            OpChoice::Cut => {
                let start = self
                    .start
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("'{}' is not a number", self.start.trim()))?;
                let end = self
                    .end
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("'{}' is not a number", self.end.trim()))?;
                BatchOp::Cut { start, end }
            }
            OpChoice::Filter => BatchOp::Filter {
                names: self
                    .names
                    .split(['\n', ','])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            },
            OpChoice::Export => BatchOp::Export,
        };
        op.validate()?;
        Ok(op)
    }

    /// Draws the tab. `queue` is owned by the app so it can be saved with the
    /// rest of the session.
    pub fn show(&mut self, ui: &mut egui::Ui, queue: &mut BatchQueue) {
        if let Some(run) = &mut self.run {
            run.poll();
            if run.finished() {
                self.last_summary = Some(summarise(run.outcomes()));
            }
        }
        let running = self
            .run
            .as_ref()
            .is_some_and(|r| !r.finished());

        ui.heading("Batch");
        ui.label(
            "Queue several measurements and apply one operation to all of them. A file that \
             fails is reported with its reason and the rest of the queue still runs.",
        );
        ui.separator();

        self.queue_controls(ui, queue, running);
        ui.separator();
        self.operation_controls(ui, running);
        ui.separator();
        self.run_controls(ui, queue, running);
        ui.separator();
        self.outcomes(ui);
    }

    /// Add, remove, and the list of what is queued.
    fn queue_controls(&mut self, ui: &mut egui::Ui, queue: &mut BatchQueue, running: bool) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Add Files\u{2026}"))
                .on_hover_text("Queue one or more measurements")
                .clicked()
            {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("MF4", &["mf4", "MF4"])
                    .pick_files()
                {
                    for path in paths {
                        queue.push(&path);
                    }
                }
            }
            if ui
                .add_enabled(!running && !queue.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                queue.clear();
            }
            ui.label(format!("{} queued", queue.len()));
        });

        if queue.is_empty() {
            ui.weak("Nothing queued yet.");
            return;
        }

        // Filled in lazily: opening every queued file to count its channels
        // the moment it is added would make adding a directory as slow as
        // running the batch.
        self.fill_channel_counts(queue);

        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .id_salt("batch_queue")
            .show(ui, |ui| {
                for (index, entry) in queue.entries().iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!running, egui::Button::new("\u{2715}"))
                            .on_hover_text("Remove from the queue")
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        ui.label(entry.file_name())
                            .on_hover_text(entry.path.display().to_string());
                        match entry.channels {
                            Some(n) => ui.weak(format!("{n} channels")),
                            None => ui.weak("unreadable"),
                        };
                    });
                }
            });
        if let Some(index) = remove {
            queue.remove(index);
        }
    }

    /// Counts the channels of queued files that have not been looked at yet.
    ///
    /// One file per frame: the count is a convenience, and opening a hundred
    /// files in one frame to fill a column would be the freeze this tab exists
    /// to avoid. A file that will not open shows no count rather than an
    /// error — the run reports that properly.
    fn fill_channel_counts(&self, queue: &mut BatchQueue) {
        let Some(entry) = queue.entries_mut().iter_mut().find(|e| !e.inspected) else {
            return;
        };
        entry.inspected = true;
        entry.channels = falcon_mdf::Mf4File::open_buffered(&entry.path)
            .ok()
            .map(|f| f.channel_count());
    }

    /// The operation and its parameters.
    fn operation_controls(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Operation:");
            for choice in OpChoice::ALL {
                ui.add_enabled_ui(!running, |ui| {
                    ui.selectable_value(&mut self.choice, choice, choice.label());
                });
            }
        });

        ui.add_enabled_ui(!running, |ui| match self.choice {
            OpChoice::Cut => {
                ui.horizontal(|ui| {
                    ui.label("From");
                    ui.add(egui::TextEdit::singleline(&mut self.start).desired_width(80.0));
                    ui.label("to");
                    ui.add(egui::TextEdit::singleline(&mut self.end).desired_width(80.0));
                    ui.label("seconds");
                });
                ui.weak("Writes <name>.cut.mf4 beside each input, or in the output folder.");
            }
            OpChoice::Filter => {
                ui.label("Channels to keep, one per line or comma-separated:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.names)
                        .desired_rows(3)
                        .hint_text("VehicleSpeed\nEngineRPM"),
                );
                ui.weak("Writes <name>.filtered.mf4. A name a file does not have is reported.");
            }
            OpChoice::Export => {
                ui.weak("Writes every channel of each file to <name>.export.csv.");
            }
        });

        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Output Folder\u{2026}"))
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.out_dir = Some(dir);
                }
            }
            match &self.out_dir {
                Some(dir) => {
                    ui.label(dir.display().to_string());
                    if ui.add_enabled(!running, egui::Button::new("Reset")).clicked() {
                        self.out_dir = None;
                    }
                }
                None => {
                    ui.weak("beside each input file");
                }
            }
        });
    }

    /// Run, cancel, and the progress bar.
    fn run_controls(&mut self, ui: &mut egui::Ui, queue: &BatchQueue, running: bool) {
        let op = self.current_op();

        ui.horizontal_wrapped(|ui| {
            let ready = !running && !queue.is_empty() && op.is_ok();
            if ui
                .add_enabled(ready, egui::Button::new("Run Batch"))
                .clicked()
            {
                if let Ok(op) = &op {
                    self.run_total = queue.len();
                    self.last_summary = None;
                    self.run = Some(spawn(
                        queue.paths(),
                        op.clone(),
                        self.out_dir.clone(),
                        ui.ctx().clone(),
                    ));
                }
            }
            if ui
                .add_enabled(running, egui::Button::new("Cancel"))
                .on_hover_text("Stop after the file being processed")
                .clicked()
            {
                if let Some(run) = &self.run {
                    run.cancel();
                }
            }
            if let Err(reason) = &op {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), reason);
            }
        });

        let Some(run) = &self.run else {
            return;
        };
        let total = self.run_total;
        ui.add(
            egui::ProgressBar::new(run.fraction(total))
                .show_percentage()
                .desired_width(320.0),
        );
        match run.current() {
            Some((index, total, path)) => {
                ui.weak(format!(
                    "{} of {total}: {}",
                    index + 1,
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            None if run.finished() => {
                if let Some(summary) = &self.last_summary {
                    ui.label(summary);
                }
            }
            None => {
                ui.weak("starting\u{2026}");
            }
        }
    }

    /// One line per finished file: what was written, or why nothing was.
    fn outcomes(&mut self, ui: &mut egui::Ui) {
        let Some(run) = &self.run else {
            return;
        };
        if run.outcomes().is_empty() {
            return;
        }

        egui::ScrollArea::vertical()
            .max_height(240.0)
            .id_salt("batch_outcomes")
            .show(ui, |ui| {
                for outcome in run.outcomes() {
                    ui.horizontal_wrapped(|ui| {
                        match &outcome.result {
                            Ok(message) => {
                                ui.label("\u{2713}");
                                ui.strong(outcome.file_name());
                                ui.weak(message);
                            }
                            // A failure is the whole reason this list exists,
                            // so it says which file and why, in the colour the
                            // rest of the viewer uses for a failed open.
                            Err(reason) => {
                                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), "\u{2717}");
                                ui.strong(outcome.file_name());
                                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), reason);
                            }
                        }
                    });
                }
            });
    }
}
