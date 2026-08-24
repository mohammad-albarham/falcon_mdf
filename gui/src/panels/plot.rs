//! The plot panel: every plotted channel decimated against its master, in
//! overlay or stacked mode, with cursor readouts, zoom and pan. `egui_plot`
//! gives zoom/pan for free; this panel's job is feeding it decimated points
//! instead of raw samples (see `crate::decimate`), keeping one decode per
//! channel alive across frames, and surfacing failed decodes as text rather
//! than silence.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use crate::computed::{evaluate_computed_channel, ComputedDef};
use crate::decimate::decimate_min_max_gaps;
use egui_plot::{Legend, Line, Plot, VLine};
use falcon_mdf::blocks::EvSyncType;
use falcon_mdf::Mf4File;

use crate::job::Job;
use crate::model::{ChannelLoc, LoadedFile, PlottedChannel, PALETTE};
use crate::signal_loader::{decode_channel, spawn_signal_load, ChannelSignal, SignalLoadResult};

/// One plotted channel's decode state.
enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    /// Decode failed — or the channel declared itself unreadable before a
    /// decode was even attempted. Either way the message is shown in the
    /// plot area; a failed channel is never silently absent.
    Failed(String),
}

/// Identifies a plotted series (either a channel from the file or a computed expression).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeriesKey {
    File(ChannelLoc),
    Computed(usize),
}

/// A series ready to be drawn in the plot panel.
pub struct PlottedSeries<'a> {
    pub key: SeriesKey,
    pub name: &'a str,
    pub color: egui::Color32,
    pub width: f32,
    pub signal: &'a ChannelSignal,
}

/// The last decimation computed for one channel, so a frame where the view
/// hasn't moved doesn't re-scan the signal. Keyed on the visible time range
/// and the pixel width `egui_plot` reported: any change to either means
/// different pixel columns, so the cache is stale. (The channel itself is
/// the `HashMap` key.)
struct DecimationCache {
    x_range: (f64, f64),
    n_columns: usize,
    segments: Vec<Vec<[f64; 2]>>,
}

/// All visible channels on one pair of axes, or one subplot per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlotMode {
    Overlay,
    Stacked,
}

/// Whether the x axis displays relative seconds from start or absolute wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeMode {
    #[default]
    Relative,
    Absolute,
}

/// Minimum, maximum, mean, and sample counts over a region between cursors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionStats {
    pub count: usize,
    pub excluded: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

/// Measurement cursor values and differences for a single signal at cursor positions A and B.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorMeasurement {
    /// Sample value at cursor A, or None if cursor A is not placed.
    pub value_a: Option<f64>,
    /// Whether the sample at cursor A is valid.
    pub valid_a: bool,
    /// Sample value at cursor B, or None if cursor B is not placed.
    pub value_b: Option<f64>,
    /// Whether the sample at cursor B is valid.
    pub valid_b: bool,
    /// Difference in timestamp between cursor B and cursor A (B - A).
    pub delta_t: Option<f64>,
    /// Difference in signal value between cursor B and cursor A (val_B - val_A),
    /// or None if either cursor is not placed or either sample is invalid.
    pub delta_y: Option<f64>,
}

/// Precomputed statistics over the region between cursor A and cursor B.
#[derive(Clone, Debug)]
struct RegionStatsCache {
    cursor_a: f64,
    cursor_b: f64,
    stats: HashMap<SeriesKey, Option<RegionStats>>,
}

const CURSOR_A_COLOR: egui::Color32 = egui::Color32::from_rgb(0x33, 0x99, 0xff);
const CURSOR_B_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x99, 0x00);

pub struct PlotPanel {
    slots: HashMap<ChannelLoc, Slot>,
    caches: HashMap<SeriesKey, DecimationCache>,
    mode: PlotMode,
    time_mode: TimeMode,
    colors: HashMap<SeriesKey, egui::Color32>,
    widths: HashMap<SeriesKey, f32>,
    /// Cached statistics over the region between cursor A and cursor B.
    region_cache: Option<RegionStatsCache>,
    /// The time under the cursor as of last frame, for stacked readouts:
    /// subplots are drawn top to bottom, so a subplot above the hovered one
    /// only learns the hovered time next frame. One frame of lag on a text
    /// label is invisible, and the drawn cursor itself is real-time.
    hovered_x: Option<f64>,
    /// Outcome of the last export, shown beside the toolbar until the next
    /// one; a failed export must read as text, not as a dead button.
    export_message: Option<String>,
    /// An export running on a worker thread. The heavy part — re-decoding
    /// the channels and writing them — must not run in the frame loop, or a
    /// large channel freezes the UI for the whole export.
    export_job: Option<Job>,
    /// Whether cursor placement mode is active.
    cursor_mode: bool,
    /// Time coordinate of measurement cursor A.
    cursor_a: Option<f64>,
    /// Time coordinate of measurement cursor B.
    cursor_b: Option<f64>,
    /// Flag requesting a reset of plot bounds to full time range.
    fit_view: bool,
    /// User-defined computed channels.
    computed_defs: Vec<ComputedDef>,
    /// Whether the computed channel editor toolbar is expanded.
    show_computed_editor: bool,
    /// Pre-decoded cache of file channels used during computed evaluation.
    computed_eval_cache: HashMap<ChannelLoc, ChannelSignal>,
}

impl Default for PlotPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PlotPanel {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            caches: HashMap::new(),
            mode: PlotMode::Overlay,
            time_mode: TimeMode::Relative,
            colors: HashMap::new(),
            widths: HashMap::new(),
            region_cache: None,
            hovered_x: None,
            export_message: None,
            export_job: None,
            cursor_mode: false,
            cursor_a: None,
            cursor_b: None,
            fit_view: false,
            computed_defs: Vec::new(),
            show_computed_editor: false,
            computed_eval_cache: HashMap::new(),
        }
    }

    /// Returns the defined computed channels.
    pub fn computed_defs(&self) -> &[ComputedDef] {
        &self.computed_defs
    }

    /// Sets the defined computed channels.
    pub fn set_computed_defs(&mut self, defs: Vec<ComputedDef>) {
        self.computed_defs = defs;
        self.caches.clear();
        self.region_cache = None;
    }

    /// The time positions of measurement cursors A and B.
    pub fn cursors(&self) -> (Option<f64>, Option<f64>) {
        (self.cursor_a, self.cursor_b)
    }

    /// Sets the time positions of measurement cursors A and B.
    pub fn set_cursors(&mut self, a: Option<f64>, b: Option<f64>) {
        self.cursor_a = a;
        self.cursor_b = b;
        self.region_cache = None;
    }

    /// Starts decodes for newly plotted channels and drops everything for
    /// channels no longer plotted.
    fn sync_slots(&mut self, ui: &egui::Ui, loaded: &LoadedFile, plotted: &[PlottedChannel]) {
        for channel in plotted {
            if self.slots.contains_key(&channel.loc) {
                continue;
            }
            // A channel that already declares itself unreadable never
            // reaches the loader thread: the reason it carries *is* the
            // answer, so it becomes a failure slot directly. `unreadable()`
            // is pure metadata — no I/O.
            let loc = channel.loc;
            let ch = channel_at(&loaded.file, loc);
            let slot = match ch.unreadable() {
                Some(reason) => Slot::Failed(reason.to_string()),
                None => Slot::Loading(spawn_signal_load(
                    loaded.file.clone(),
                    loc,
                    ui.ctx().clone(),
                )),
            };
            self.slots.insert(loc, slot);
        }
        // Removing a channel drops its slot and cached decimation. Its custom
        // color and line width are dropped too so a re-added channel gets a
        // clean palette default.
        self.slots
            .retain(|loc, _| plotted.iter().any(|p| p.loc == *loc));
        self.caches.retain(|key, _| match key {
            SeriesKey::File(loc) => plotted.iter().any(|p| p.loc == *loc),
            SeriesKey::Computed(idx) => *idx < self.computed_defs.len(),
        });
        self.colors.retain(|key, _| match key {
            SeriesKey::File(loc) => plotted.iter().any(|p| p.loc == *loc),
            SeriesKey::Computed(idx) => *idx < self.computed_defs.len(),
        });
        self.widths.retain(|key, _| match key {
            SeriesKey::File(loc) => plotted.iter().any(|p| p.loc == *loc),
            SeriesKey::Computed(idx) => *idx < self.computed_defs.len(),
        });
        if let Some(cache) = &mut self.region_cache {
            cache.stats.retain(|key, _| match key {
                SeriesKey::File(loc) => plotted.iter().any(|p| p.loc == *loc),
                SeriesKey::Computed(idx) => *idx < self.computed_defs.len(),
            });
        }
    }

    fn poll(&mut self) {
        for slot in self.slots.values_mut() {
            // The receive has to happen before the slot is overwritten, so
            // the result is moved out of the borrow first.
            let result = match slot {
                Slot::Loading(rx) => Some(rx.try_recv()),
                _ => None,
            };
            match result {
                Some(Ok(SignalLoadResult::Ok(sig))) => *slot = Slot::Loaded(sig),
                Some(Ok(SignalLoadResult::Err { message })) => *slot = Slot::Failed(message),
                Some(Err(TryRecvError::Empty)) | None => {}
                Some(Err(TryRecvError::Disconnected)) => {
                    *slot = Slot::Failed("signal loader thread ended without a result".to_string());
                }
            }
        }
    }

    /// Collects the export worker's message when it finishes. A worker that
    /// ends without one is reported like every other failure in this panel:
    /// as text, not as silence.
    fn poll_export(&mut self) {
        if let Some(job) = &self.export_job {
            if let Some(message) = job.poll() {
                self.export_message = Some(message);
                self.export_job = None;
            }
        }
    }

    /// The visible, decoded channels the export should cover. A channel that
    /// is plotted but not decoded yet is left out rather than decoded here —
    /// the loader already owns that work.
    fn exportable_locs(&self, plotted: &[PlottedChannel]) -> Vec<ChannelLoc> {
        plotted
            .iter()
            .filter(|p| p.visible)
            .filter(|p| matches!(self.slots.get(&p.loc), Some(Slot::Loaded(_))))
            .map(|p| p.loc)
            .collect()
    }

    fn start_csv_export(&mut self, ui: &egui::Ui, loaded: &LoadedFile, plotted: &[PlottedChannel]) {
        let locs = self.exportable_locs(plotted);
        if locs.is_empty() {
            self.export_message = Some("nothing decoded to export yet".to_string());
            return;
        }
        let default = format!(
            "{}.csv",
            sanitized_file_name(&channel_at(&loaded.file, locs[0]).name)
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(&default)
            .save_file()
        else {
            return;
        };
        let file = Arc::clone(&loaded.file);
        self.export_message = None;
        self.export_job = Some(Job::spawn(ui.ctx(), move || {
            run_csv_export(&file, &locs, &path)
        }));
    }

    fn start_mf4_export(&mut self, ui: &egui::Ui, loaded: &LoadedFile, plotted: &[PlottedChannel]) {
        let locs = self.exportable_locs(plotted);
        if locs.is_empty() {
            self.export_message = Some("nothing decoded to export yet".to_string());
            return;
        }
        let default = format!(
            "{}.mf4",
            sanitized_file_name(&channel_at(&loaded.file, locs[0]).name)
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MF4", &["mf4", "MF4"])
            .set_file_name(&default)
            .save_file()
        else {
            return;
        };
        let file = Arc::clone(&loaded.file);
        // The exported file keeps the source's start time, so a re-export
        // keeps its provenance.
        let start_time_ns = loaded.file.start_time().timestamp_ns;
        self.export_message = None;
        self.export_job = Some(Job::spawn(ui.ctx(), move || {
            run_mf4_export(&file, &locs, start_time_ns, &path)
        }));
    }

    fn show_computed_controls(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Computed channels");
                if ui.button("+ Add channel").clicked() {
                    let n = self.computed_defs.len() + 1;
                    self.computed_defs.push(ComputedDef {
                        name: format!("calc_{n}"),
                        expression: String::new(),
                        unit: String::new(),
                    });
                }
            });

            let mut to_remove = None;
            for (idx, def) in self.computed_defs.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.add(egui::TextEdit::singleline(&mut def.name).desired_width(80.0));
                    ui.label("Expr:");
                    ui.add(
                        egui::TextEdit::singleline(&mut def.expression)
                            .desired_width(220.0)
                            .hint_text("e.g. Speed * 3.6 or [FL] - [FR]"),
                    );
                    ui.label("Unit:");
                    ui.add(egui::TextEdit::singleline(&mut def.unit).desired_width(50.0));

                    if ui
                        .button("\u{1f5d1}")
                        .on_hover_text("Delete this computed channel")
                        .clicked()
                    {
                        to_remove = Some(idx);
                    }
                });
            }
            if let Some(idx) = to_remove {
                self.computed_defs.remove(idx);
                self.caches.clear();
                self.region_cache = None;
            }

            ui.weak("Syntax: + - * /, (), numbers, channel names (use [Name] or \"Name\" if spaces/dots)");
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile, plotted: &[PlottedChannel]) {
        self.sync_slots(ui, loaded, plotted);
        self.poll();
        self.poll_export();

        if plotted.is_empty() && self.computed_defs.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("View:");
                ui.selectable_value(&mut self.mode, PlotMode::Overlay, "Overlay");
                ui.selectable_value(&mut self.mode, PlotMode::Stacked, "Stacked");
                ui.separator();
                ui.toggle_value(&mut self.show_computed_editor, "+ Computed");
            });
            if self.show_computed_editor {
                self.show_computed_controls(ui);
            }
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("Click a channel in the list or add a computed channel to plot.");
            });
            return;
        }

        let export_busy = self.export_job.is_some();
        let mut csv_clicked = false;
        let mut mf4_clicked = false;
        // Wrapped, not a single row: the toolbar has grown past the width of
        // a narrow content pane, and a button that runs off the edge is a
        // button nobody can press.
        ui.horizontal_wrapped(|ui| {
            ui.label("View:");
            ui.selectable_value(&mut self.mode, PlotMode::Overlay, "Overlay");
            ui.selectable_value(&mut self.mode, PlotMode::Stacked, "Stacked");
            ui.separator();
            ui.label("Time:");
            ui.selectable_value(&mut self.time_mode, TimeMode::Relative, "Relative");
            ui.selectable_value(&mut self.time_mode, TimeMode::Absolute, "Absolute");
            ui.separator();
            ui.toggle_value(&mut self.cursor_mode, "Cursors");
            if ui.button("Clear cursors").clicked() {
                self.cursor_a = None;
                self.cursor_b = None;
                self.region_cache = None;
            }
            if ui.button("Fit view").clicked() {
                self.fit_view = true;
            }
            ui.separator();
            ui.toggle_value(&mut self.show_computed_editor, "+ Computed");
            ui.separator();
            // One export at a time: two workers decoding the same channels
            // into two files would be correct but pointless, and the busy
            // state keeps the toolbar honest about what is running.
            ui.add_enabled_ui(!export_busy, |ui| {
                csv_clicked = ui.button("Export CSV\u{2026}").clicked();
                mf4_clicked = ui.button("Export MF4\u{2026}").clicked();
            });
        });

        if self.show_computed_editor {
            self.show_computed_controls(ui);
        }

        if csv_clicked {
            self.start_csv_export(ui, loaded, plotted);
        }
        if mf4_clicked {
            self.start_mf4_export(ui, loaded, plotted);
        }
        if export_busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Exporting\u{2026}");
            });
        } else if let Some(message) = &self.export_message {
            ui.label(message);
        }

        let computed_defs = self.computed_defs.clone();
        let mut computed_signals: Vec<(usize, ComputedDef, Result<ChannelSignal, String>)> =
            Vec::new();
        for (idx, def) in computed_defs.into_iter().enumerate() {
            if def.name.trim().is_empty() && def.expression.trim().is_empty() {
                continue;
            }
            let res = evaluate_computed_channel(&def, &loaded.file, &mut self.computed_eval_cache);
            computed_signals.push((idx, def, res));
        }

        // Failures are listed inline, right where the channel's line would
        // be — never an empty plot with no explanation.
        for channel in plotted.iter().filter(|p| p.visible) {
            if let Some(Slot::Failed(message)) = self.slots.get(&channel.loc) {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("{}: {message}", channel.name),
                );
            }
        }

        // Failures for computed channels
        for (_, def, res) in &computed_signals {
            if let Err(message) = res {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Computed '{}': {message}", def.name),
                );
            }
        }

        let any_loading = plotted
            .iter()
            .any(|p| p.visible && matches!(self.slots.get(&p.loc), Some(Slot::Loading(_))));
        if any_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Decoding channel\u{2026}");
            });
        }

        // Build unified drawable series list: real channels followed by computed channels
        let mut drawable: Vec<PlottedSeries> = Vec::new();
        for p in plotted.iter().filter(|p| p.visible) {
            if let Some(Slot::Loaded(signal)) = self.slots.get(&p.loc) {
                if !signal.times.is_empty() {
                    let key = SeriesKey::File(p.loc);
                    let color = *self.colors.entry(key).or_insert(p.color);
                    let width = *self.widths.entry(key).or_insert(1.5);
                    drawable.push(PlottedSeries {
                        key,
                        name: &p.name,
                        color,
                        width,
                        signal,
                    });
                }
            }
        }

        for (idx, def, res) in &computed_signals {
            if let Ok(signal) = res {
                if !signal.times.is_empty() {
                    let key = SeriesKey::Computed(*idx);
                    let default_color = PALETTE[(plotted.len() + *idx) % PALETTE.len()];
                    let color = *self.colors.entry(key).or_insert(default_color);
                    let width = *self.widths.entry(key).or_insert(1.5);
                    drawable.push(PlottedSeries {
                        key,
                        name: &def.name,
                        color,
                        width,
                        signal,
                    });
                }
            }
        }

        if drawable.is_empty() {
            if !any_loading
                && (plotted.iter().any(|p| p.visible) || !self.computed_defs.is_empty())
            {
                ui.label("The plotted channels have no samples or failed to evaluate.");
            }
            return;
        }

        // Per-signal controls in the legend area: color picker and line width.
        // Editing either affects draw time only, so cached decimation remains valid.
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Signals:").weak());
            for item in &drawable {
                let color = self.colors.entry(item.key).or_insert(item.color);
                let width = self.widths.entry(item.key).or_insert(item.width);
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgba(color);
                    ui.label(item.name);
                    ui.add(
                        egui::DragValue::new(width)
                            .speed(0.1)
                            .range(1.0..=4.0)
                            .prefix("w: "),
                    );
                });
                ui.add_space(6.0);
            }
        });

        // The union of all visible time ranges: the shared X axis has to
        // cover every channel, including ones whose master starts later or
        // ends earlier than the first's.
        // `drawable` holds only non-empty signals, and the range re-checks
        // that here rather than trusting the filter that built it.
        let full_range = drawable
            .iter()
            .filter_map(|s| Some((*s.signal.times.first()?, *s.signal.times.last()?)))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (a, b)| {
                (lo.min(a), hi.max(b))
            });

        // Events synchronised to time become markers on the shared X axis;
        // angle, distance and index events do not belong on a time axis and
        // stay in the metadata panel's list. Capped, so a file thick with
        // triggers cannot flood the plot and its legend.
        let event_marks: Vec<(String, f64)> = loaded
            .file
            .events()
            .iter()
            .filter(|e| e.sync_type == EvSyncType::Time)
            .take(20)
            .map(|e| {
                let label = if e.name.is_empty() {
                    format!("{:?}", e.event_type)
                } else {
                    e.name.clone()
                };
                (format!("\u{2691} {label}"), e.position())
            })
            .collect();

        let fit_view = self.fit_view;
        if fit_view {
            self.caches.clear();
        }

        // Region statistics calculation: compute and cache when both cursors
        // are placed and the range or channel set changes.
        if let (Some(a), Some(b)) = (self.cursor_a, self.cursor_b) {
            let cache_valid = match &self.region_cache {
                Some(cache) => {
                    (cache.cursor_a - a).abs() < 1e-9
                        && (cache.cursor_b - b).abs() < 1e-9
                        && drawable
                            .iter()
                            .all(|item| cache.stats.contains_key(&item.key))
                }
                None => false,
            };
            if !cache_valid {
                let mut stats = HashMap::new();
                for item in &drawable {
                    let st = region_stats(
                        &item.signal.times,
                        &item.signal.values,
                        item.signal.valid.as_deref(),
                        a,
                        b,
                    );
                    stats.insert(item.key, st);
                }
                self.region_cache = Some(RegionStatsCache {
                    cursor_a: a,
                    cursor_b: b,
                    stats,
                });
            }
        } else {
            self.region_cache = None;
        }

        let start_time_ns = loaded.file.start_time().timestamp_ns;
        let time_mode = self.time_mode;
        let caches = &mut self.caches;
        let region_cache = self.region_cache.as_ref();
        let hovered_x = &mut self.hovered_x;
        let cursor_a = &mut self.cursor_a;
        let cursor_b = &mut self.cursor_b;
        let cursor_mode = self.cursor_mode;

        match self.mode {
            PlotMode::Overlay => Self::show_overlay(
                ui,
                caches,
                &drawable,
                full_range,
                &event_marks,
                cursor_mode,
                cursor_a,
                cursor_b,
                fit_view,
                time_mode,
                start_time_ns,
                region_cache,
            ),
            PlotMode::Stacked => Self::show_stacked(
                ui,
                caches,
                hovered_x,
                &drawable,
                full_range,
                &event_marks,
                cursor_mode,
                cursor_a,
                cursor_b,
                fit_view,
                time_mode,
                start_time_ns,
                region_cache,
            ),
        }

        self.fit_view = false;
    }

    /// Associated functions rather than `&mut self` methods: `drawable`
    /// borrows `self.slots` at the call site, and these only need the other
    /// fields, so keeping them separate avoids borrowing all of `self`.
    #[allow(clippy::too_many_arguments)]
    fn show_overlay(
        ui: &mut egui::Ui,
        caches: &mut HashMap<SeriesKey, DecimationCache>,
        drawable: &[PlottedSeries],
        full_range: (f64, f64),
        event_marks: &[(String, f64)],
        cursor_mode: bool,
        cursor_a: &mut Option<f64>,
        cursor_b: &mut Option<f64>,
        fit_view: bool,
        time_mode: TimeMode,
        start_time_ns: i64,
        region_cache: Option<&RegionStatsCache>,
    ) {
        let mut hovered_time = None;
        let first = drawable[0].signal;

        let x_label = match time_mode {
            TimeMode::Relative => axis_label(&first.time_name, &first.time_unit),
            TimeMode::Absolute => format!("{} [UTC]", first.time_name),
        };

        let mut plot = Plot::new("overlay_plot")
            .legend(Legend::default())
            .x_axis_label(x_label);

        if time_mode == TimeMode::Absolute {
            plot = plot
                .x_axis_formatter(move |mark, _range| absolute_label(start_time_ns, mark.value));
        }

        if fit_view {
            plot = plot.reset();
        }

        let response = plot.show(ui, |plot_ui| {
            let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;
            let bounds = plot_ui.plot_bounds();
            for item in drawable {
                // On a channel's first frame the plot still reports its
                // default (0..1) bounds, so decimate against the full
                // range until real bounds exist.
                let x_range = if caches.contains_key(&item.key) {
                    (bounds.min()[0], bounds.max()[0])
                } else {
                    full_range
                };
                // One `Line` per valid segment (see decimate_min_max_gaps).
                // egui_plot's legend merges same-named, same-colored
                // items into one entry, so the gap split doesn't
                // multiply legend rows. The legend entry names the unit,
                // since overlay mode is where channels with different
                // units share one axis and the legend is the only place
                // to say which line is which.
                let legend_name = axis_label(item.name, &item.signal.unit);
                for segment in segments_for(caches, item.key, item.signal, x_range, n_columns) {
                    plot_ui.line(
                        Line::new(legend_name.clone(), segment)
                            .color(item.color)
                            .width(item.width),
                    );
                }
            }
            for (name, x) in event_marks {
                plot_ui.vline(VLine::new(name.clone(), *x).stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(150, 150, 90),
                )));
            }
            if let Some(a) = *cursor_a {
                plot_ui.vline(VLine::new("A", a).stroke(egui::Stroke::new(1.5, CURSOR_A_COLOR)));
            }
            if let Some(b) = *cursor_b {
                plot_ui.vline(VLine::new("B", b).stroke(egui::Stroke::new(1.5, CURSOR_B_COLOR)));
            }
            if plot_ui.response().hovered() {
                if let Some(pos) = plot_ui.pointer_coordinate() {
                    hovered_time = Some(pos.x);
                    plot_ui.vline(
                        VLine::new("cursor", pos.x)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY)),
                    );
                }
            }
            plot_ui.pointer_coordinate()
        });

        if cursor_mode && response.response.clicked() {
            if let Some(pos) = response.inner {
                if ui.input(|i| i.modifiers.shift) {
                    *cursor_b = Some(pos.x);
                } else {
                    *cursor_a = Some(pos.x);
                }
            }
        }

        match hovered_time {
            Some(t) => {
                for item in drawable {
                    ui.horizontal(|ui| {
                        ui.colored_label(item.color, "\u{25cf}");
                        ui.label(readout(item.signal, t, time_mode, start_time_ns));
                    });
                }
            }
            None => {
                if cursor_a.is_none() && cursor_b.is_none() && !cursor_mode {
                    ui.label("Hover the plot for a value readout.");
                }
            }
        }

        show_cursor_readout(
            ui,
            drawable,
            *cursor_a,
            *cursor_b,
            cursor_mode,
            time_mode,
            start_time_ns,
            region_cache,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn show_stacked(
        ui: &mut egui::Ui,
        caches: &mut HashMap<SeriesKey, DecimationCache>,
        hovered_x: &mut Option<f64>,
        drawable: &[PlottedSeries],
        full_range: (f64, f64),
        event_marks: &[(String, f64)],
        cursor_mode: bool,
        cursor_a: &mut Option<f64>,
        cursor_b: &mut Option<f64>,
        fit_view: bool,
        time_mode: TimeMode,
        start_time_ns: i64,
        region_cache: Option<&RegionStatsCache>,
    ) {
        // One subplot per channel, X-linked so zoom/pan stay in sync while
        // each keeps its own Y auto-bounds. Stacking is also the honest
        // answer for channels with different units: egui_plot 0.36 has no
        // per-series second Y axis (axes exist only as widgets), so instead
        // of silently plotting volts against RPM on one axis, each channel
        // gets its own scale.
        let n = drawable.len();
        let readout_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let height = ((ui.available_height() - readout_height * n as f32) / n as f32).max(1.0);
        let last = n - 1;
        let mut hovered_now = None;

        for (index, item) in drawable.iter().enumerate() {
            let signal = item.signal;
            let plot_id = match item.key {
                SeriesKey::File(loc) => (
                    "stacked_plot_file",
                    loc.data_group_index,
                    loc.channel_group_index,
                    loc.channel_index,
                ),
                SeriesKey::Computed(idx) => (
                    "stacked_plot_computed",
                    usize::MAX,
                    0,
                    idx,
                ),
            };
            let mut plot = Plot::new(plot_id)
                .link_axis("stacked_x", egui::Vec2b::new(true, false))
                .link_cursor("stacked_x", egui::Vec2b::new(true, false))
                .height(height)
                .include_x(full_range.0)
                .include_x(full_range.1)
                .y_axis_label(axis_label(item.name, &signal.unit));
            // Only the bottom subplot names the X axis; every subplot shares
            // it, and repeating the label just spends vertical space.
            if index == last {
                let x_label = match time_mode {
                    TimeMode::Relative => axis_label(&signal.time_name, &signal.time_unit),
                    TimeMode::Absolute => format!("{} [UTC]", signal.time_name),
                };
                plot = plot.x_axis_label(x_label);
            }
            if time_mode == TimeMode::Absolute {
                plot = plot.x_axis_formatter(move |mark, _range| {
                    absolute_label(start_time_ns, mark.value)
                });
            }
            if fit_view {
                plot = plot.reset();
            }

            let response = plot.show(ui, |plot_ui| {
                let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;
                let bounds = plot_ui.plot_bounds();
                let x_range = if caches.contains_key(&item.key) {
                    (bounds.min()[0], bounds.max()[0])
                } else {
                    full_range
                };
                for segment in segments_for(caches, item.key, signal, x_range, n_columns) {
                    plot_ui.line(
                        Line::new(item.name.to_string(), segment)
                            .color(item.color)
                            .width(item.width),
                    );
                }
                for (name, x) in event_marks {
                    plot_ui.vline(VLine::new(name.clone(), *x).stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(150, 150, 90),
                    )));
                }
                if let Some(a) = *cursor_a {
                    plot_ui
                        .vline(VLine::new("A", a).stroke(egui::Stroke::new(1.5, CURSOR_A_COLOR)));
                }
                if let Some(b) = *cursor_b {
                    plot_ui
                        .vline(VLine::new("B", b).stroke(egui::Stroke::new(1.5, CURSOR_B_COLOR)));
                }
                // No manual VLine here: plots in a cursor link group draw
                // each other's vertical cursor automatically, which is the
                // gray line the overlay mode draws by hand.
                plot_ui.pointer_coordinate()
            });
            if response.response.hovered() {
                if let Some(pos) = response.inner {
                    hovered_now = Some(pos.x);
                }
            }
            if cursor_mode && response.response.clicked() {
                if let Some(pos) = response.inner {
                    if ui.input(|i| i.modifiers.shift) {
                        *cursor_b = Some(pos.x);
                    } else {
                        *cursor_a = Some(pos.x);
                    }
                }
            }

            match hovered_now.or(*hovered_x) {
                Some(t) => {
                    ui.horizontal(|ui| {
                        ui.colored_label(item.color, "\u{25cf}");
                        ui.label(readout(signal, t, time_mode, start_time_ns));
                    });
                }
                None => {
                    if cursor_a.is_none() && cursor_b.is_none() && !cursor_mode {
                        ui.label("Hover the plot for a value readout.");
                    }
                }
            }
        }
        *hovered_x = hovered_now;

        show_cursor_readout(
            ui,
            drawable,
            *cursor_a,
            *cursor_b,
            cursor_mode,
            time_mode,
            start_time_ns,
            region_cache,
        );
    }
}

/// Decimated segments for one channel at the current view, from the cache
/// when the view hasn't moved since last frame.
fn segments_for(
    caches: &mut HashMap<SeriesKey, DecimationCache>,
    key: SeriesKey,
    signal: &ChannelSignal,
    x_range: (f64, f64),
    n_columns: usize,
) -> Vec<Vec<[f64; 2]>> {
    match caches.get(&key) {
        Some(c) if c.x_range == x_range && c.n_columns == n_columns => c.segments.clone(),
        _ => {
            let segments = decimate_min_max_gaps(
                &signal.times,
                &signal.values,
                signal.valid.as_deref(),
                x_range,
                n_columns,
            );
            caches.insert(
                key,
                DecimationCache {
                    x_range,
                    n_columns,
                    segments: segments.clone(),
                },
            );
            segments
        }
    }
}

/// Returns the nearest sample index and whether it is valid for `signal` at time `t`.
fn sample_at(signal: &ChannelSignal, t: f64) -> (usize, bool) {
    let i = nearest_index(&signal.times, t);
    let valid = match &signal.valid {
        Some(v) => v.get(i).copied().unwrap_or(true),
        None => true,
    };
    (i, valid)
}

/// The readout line for one signal at hovered time `t`. A sample the file
/// marks invalid is gapped out of the plot, so the readout must not quietly
/// show the garbage value the record held there either. Names the channel
/// and its unit, since several readouts are shown together once more than
/// one channel is plotted.
fn readout(signal: &ChannelSignal, t: f64, time_mode: TimeMode, start_time_ns: i64) -> String {
    let (i, valid) = sample_at(signal, t);
    let t_str = match time_mode {
        TimeMode::Relative => format!("{:.6}", signal.times[i]),
        TimeMode::Absolute => absolute_label(start_time_ns, signal.times[i]),
    };
    if valid {
        let value = axis_label(&format!("{:.6}", signal.values[i]), &signal.unit);
        format!("{}: t = {}    value = {}", signal.name, t_str, value)
    } else {
        format!("{}: t = {}    (sample marked invalid)", signal.name, t_str)
    }
}

/// Renders the measurement cursor readout table and the region statistics
/// table under the plot when cursors are placed or cursor mode is enabled.
#[allow(clippy::too_many_arguments)]
fn show_cursor_readout(
    ui: &mut egui::Ui,
    drawable: &[PlottedSeries],
    cursor_a: Option<f64>,
    cursor_b: Option<f64>,
    cursor_mode: bool,
    time_mode: TimeMode,
    start_time_ns: i64,
    region_cache: Option<&RegionStatsCache>,
) {
    if cursor_a.is_none() && cursor_b.is_none() {
        if cursor_mode {
            ui.label("Click in the plot to place cursor A; Shift-click to place cursor B.");
        }
        return;
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Measurement cursors");
        if cursor_a.is_none() {
            ui.label(egui::RichText::new("(click plot to place A)").weak());
        } else if cursor_b.is_none() {
            ui.label(egui::RichText::new("(Shift-click plot to place B)").weak());
        }
    });

    egui::Grid::new("measurement_cursor_grid")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Signal");
            ui.colored_label(CURSOR_A_COLOR, "Cursor A");
            ui.colored_label(CURSOR_B_COLOR, "Cursor B");
            ui.strong("\u{0394} (B \u{2212} A)");
            ui.end_row();

            let first_time_unit = &drawable[0].signal.time_unit;
            ui.label("Time");
            ui.label(match cursor_a {
                Some(t) => match time_mode {
                    TimeMode::Relative => axis_label(&format!("{:.6}", t), first_time_unit),
                    TimeMode::Absolute => absolute_label(start_time_ns, t),
                },
                None => "\u{2014}".to_string(),
            });
            ui.label(match cursor_b {
                Some(t) => match time_mode {
                    TimeMode::Relative => axis_label(&format!("{:.6}", t), first_time_unit),
                    TimeMode::Absolute => absolute_label(start_time_ns, t),
                },
                None => "\u{2014}".to_string(),
            });
            ui.label(match (cursor_a, cursor_b) {
                (Some(a), Some(b)) => axis_label(&format!("{:.6}", b - a), first_time_unit),
                _ => "\u{2014}".to_string(),
            });
            ui.end_row();

            for item in drawable {
                let signal = item.signal;
                ui.horizontal(|ui| {
                    ui.colored_label(item.color, "\u{25cf}");
                    ui.label(axis_label(item.name, &signal.unit));
                });

                let m = cursor_measurement(
                    &signal.times,
                    &signal.values,
                    signal.valid.as_deref(),
                    cursor_a,
                    cursor_b,
                );

                ui.label(match (m.value_a, m.valid_a) {
                    (Some(v), true) => {
                        axis_label(&format!("{:.6}", v), &signal.unit)
                    }
                    (Some(_), false) => "(invalid)".to_string(),
                    (None, _) => "\u{2014}".to_string(),
                });

                ui.label(match (m.value_b, m.valid_b) {
                    (Some(v), true) => {
                        axis_label(&format!("{:.6}", v), &signal.unit)
                    }
                    (Some(_), false) => "(invalid)".to_string(),
                    (None, _) => "\u{2014}".to_string(),
                });

                ui.label(match m.delta_y {
                    Some(delta) => {
                        axis_label(&format!("{:.6}", delta), &signal.unit)
                    }
                    None => "\u{2014}".to_string(),
                });
                ui.end_row();
            }
        });

    if let (Some(a), Some(b)) = (cursor_a, cursor_b) {
        let (t_min, t_max) = if a <= b { (a, b) } else { (b, a) };
        let dt = (b - a).abs();
        let first_time_unit = &drawable[0].signal.time_unit;

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Region statistics");
            ui.weak(match time_mode {
                TimeMode::Relative => {
                    format!(
                        "({:.6} {} \u{2026} {:.6} {}, \u{0394}t = {:.6} {})",
                        t_min, first_time_unit, t_max, first_time_unit, dt, first_time_unit
                    )
                }
                TimeMode::Absolute => {
                    format!(
                        "({} \u{2026} {}, \u{0394}t = {:.6} {})",
                        absolute_label(start_time_ns, t_min),
                        absolute_label(start_time_ns, t_max),
                        dt,
                        first_time_unit
                    )
                }
            });
        });

        egui::Grid::new("region_stats_grid")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Signal");
                ui.strong("Samples");
                ui.strong("Min");
                ui.strong("Max");
                ui.strong("Mean");
                ui.strong("\u{0394} (B \u{2212} A)");
                ui.end_row();

                for item in drawable {
                    let signal = item.signal;
                    ui.horizontal(|ui| {
                        ui.colored_label(item.color, "\u{25cf}");
                        ui.label(axis_label(item.name, &signal.unit));
                    });

                    let st =
                        region_cache.and_then(|c| c.stats.get(&item.key).copied().flatten());

                    match st {
                        Some(st) => {
                            if st.excluded > 0 {
                                ui.label(format!("{} ({} excluded)", st.count, st.excluded));
                            } else {
                                ui.label(format!("{}", st.count));
                            }
                            ui.label(axis_label(&format!("{:.6}", st.min), &signal.unit));
                            ui.label(axis_label(&format!("{:.6}", st.max), &signal.unit));
                            ui.label(axis_label(&format!("{:.6}", st.mean), &signal.unit));

                            if st.count == 1 {
                                ui.label(axis_label(&format!("{:.6}", 0.0), &signal.unit));
                            } else {
                                let (ia, va) = sample_at(signal, a);
                                let (ib, vb) = sample_at(signal, b);
                                if va && vb {
                                    let delta = signal.values[ib] - signal.values[ia];
                                    ui.label(axis_label(&format!("{:.6}", delta), &signal.unit));
                                } else {
                                    ui.label("\u{2014}");
                                }
                            }
                        }
                        None => {
                            ui.label("0");
                            ui.label("(no samples in region)");
                            ui.label("\u{2014}");
                            ui.label("\u{2014}");
                            ui.label("\u{2014}");
                        }
                    }
                    ui.end_row();
                }
            });
    }
}

/// Computes minimum, maximum, and mean values over all valid samples of a
/// signal whose timestamps fall in `[from, to]` (inclusive, order-independent).
/// Returns `None` if no valid samples fall inside the region.
pub fn region_stats(
    times: &[f64],
    values: &[f64],
    valid: Option<&[bool]>,
    from: f64,
    to: f64,
) -> Option<RegionStats> {
    let len = times.len().min(values.len());
    if len == 0 {
        return None;
    }
    let times = &times[..len];
    let values = &values[..len];

    let (t_min, t_max) = if from <= to { (from, to) } else { (to, from) };
    let start = times.partition_point(|&t| t < t_min);
    let end = times.partition_point(|&t| t <= t_max);
    if start >= end {
        return None;
    }

    let mut count = 0usize;
    let mut excluded = 0usize;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0;

    let val_slice = &values[start..end];
    let valid_slice = valid.map(|v| {
        let v_start = start.min(v.len());
        let v_end = end.min(v.len());
        &v[v_start..v_end]
    });

    for (offset, &val) in val_slice.iter().enumerate() {
        let is_valid = match valid_slice {
            Some(v) => v.get(offset).copied().unwrap_or(true),
            None => true,
        };
        if !is_valid {
            excluded += 1;
            continue;
        }
        if val.is_nan() {
            excluded += 1;
            continue;
        }
        count += 1;
        if val < min {
            min = val;
        }
        if val > max {
            max = val;
        }
        sum += val;
    }

    if count == 0 {
        return None;
    }

    let mean = sum / count as f64;
    Some(RegionStats {
        count,
        excluded,
        min,
        max,
        mean,
    })
}

/// Formats a recording start time (nanoseconds since Unix epoch) plus an offset
/// in seconds as a wall-clock UTC timestamp with millisecond precision:
/// `"YYYY-MM-DD HH:MM:SS.mmm"`.
pub fn absolute_label(start_time_ns: i64, offset_seconds: f64) -> String {
    let start_secs = start_time_ns.div_euclid(1_000_000_000);
    let start_nanos = start_time_ns.rem_euclid(1_000_000_000);

    let offset_whole_secs = offset_seconds.floor() as i64;
    let offset_frac_secs = offset_seconds - (offset_whole_secs as f64);
    let offset_nanos = (offset_frac_secs * 1_000_000_000.0).round() as i64;

    let mut total_nanos = start_nanos + offset_nanos;
    let mut total_secs = start_secs + offset_whole_secs + total_nanos.div_euclid(1_000_000_000);
    total_nanos = total_nanos.rem_euclid(1_000_000_000);

    let mut total_millis = (total_nanos + 500_000) / 1_000_000;
    if total_millis >= 1000 {
        total_secs += 1;
        total_millis -= 1000;
    }

    let days = total_secs.div_euclid(86400);
    let day_secs = total_secs.rem_euclid(86400);

    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    let (year, month, day) = civil_from_days(days);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        year, month, day, hours, minutes, seconds, total_millis
    )
}

/// Converts days since Unix epoch (1970-01-01) to `(year, month, day)` in the
/// proleptic Gregorian calendar using Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn axis_label(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} [{unit}]")
    }
}

/// Index of the sample whose time is closest to `t`. `times` is sorted
/// ascending and non-empty.
pub fn nearest_index(times: &[f64], t: f64) -> usize {
    let i = times.partition_point(|&x| x < t);
    if i == 0 {
        return 0;
    }
    if i >= times.len() {
        return times.len() - 1;
    }
    if (times[i] - t).abs() < (t - times[i - 1]).abs() {
        i
    } else {
        i - 1
    }
}

/// Computes cursor measurements (value at A, value at B, delta_t, delta_y) for a signal.
pub fn cursor_measurement(
    times: &[f64],
    values: &[f64],
    valid: Option<&[bool]>,
    cursor_a: Option<f64>,
    cursor_b: Option<f64>,
) -> CursorMeasurement {
    let len = times.len().min(values.len());
    if len == 0 {
        return CursorMeasurement {
            value_a: None,
            valid_a: false,
            value_b: None,
            valid_b: false,
            delta_t: match (cursor_a, cursor_b) {
                (Some(a), Some(b)) => Some(b - a),
                _ => None,
            },
            delta_y: None,
        };
    }
    let times = &times[..len];
    let values = &values[..len];

    let (value_a, valid_a) = match cursor_a {
        Some(t) => {
            let i = nearest_index(times, t);
            let is_valid = match valid {
                Some(v) => v.get(i).copied().unwrap_or(true),
                None => true,
            };
            (Some(values[i]), is_valid)
        }
        None => (None, false),
    };

    let (value_b, valid_b) = match cursor_b {
        Some(t) => {
            let i = nearest_index(times, t);
            let is_valid = match valid {
                Some(v) => v.get(i).copied().unwrap_or(true),
                None => true,
            };
            (Some(values[i]), is_valid)
        }
        None => (None, false),
    };

    let delta_t = match (cursor_a, cursor_b) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };

    let delta_y = if valid_a && valid_b {
        match (value_a, value_b) {
            (Some(va), Some(vb)) => {
                if va.is_nan() || vb.is_nan() {
                    None
                } else {
                    Some(vb - va)
                }
            }
            _ => None,
        }
    } else {
        None
    };

    CursorMeasurement {
        value_a,
        valid_a,
        value_b,
        valid_b,
        delta_t,
        delta_y,
    }
}

/// The model channel behind a plotted location.
fn channel_at(file: &Mf4File, loc: ChannelLoc) -> &falcon_mdf::Channel {
    &file.data_groups()[loc.data_group_index].channel_groups[loc.channel_group_index].channels
        [loc.channel_index]
}

/// A file name the OS will accept, whatever the channel is called.
fn sanitized_file_name(name: &str) -> String {
    name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

/// The worker side of the CSV export: writes through the same `write_csv`
/// the `export_to_csv` example uses, so the app's CSV is byte for byte the
/// example's. Runs off the UI thread — on a large channel the decode that
/// `write_csv` performs internally would freeze the frame loop otherwise.
fn run_csv_export(file: &Mf4File, locs: &[ChannelLoc], path: &Path) -> String {
    let channels: Vec<&falcon_mdf::Channel> =
        locs.iter().map(|&loc| channel_at(file, loc)).collect();
    let mut out = std::io::BufWriter::new(match std::fs::File::create(path) {
        Ok(file) => file,
        Err(e) => return format!("export failed: {e}"),
    });
    match falcon_mdf::write_csv(file, &channels, &mut out)
        .and_then(|()| out.flush().map_err(Into::into))
    {
        Ok(()) => format!(
            "exported {} channel(s) to {}",
            channels.len(),
            path.display()
        ),
        Err(e) => format!("export failed: {e}"),
    }
}

/// The worker side of the MF4 export: one channel group per channel, each
/// with its own master, and validity carried over — the exported file keeps
/// the gaps the source declared rather than drawing them into the record as
/// measurements. The decode is the same one the plot shows
/// (`decode_channel`), so the export matches what is on screen.
fn run_mf4_export(file: &Mf4File, locs: &[ChannelLoc], start_time_ns: i64, path: &Path) -> String {
    let mut writer = falcon_mdf::Mf4Writer::with_start_time_ns(start_time_ns);
    for &loc in locs {
        let signal = match decode_channel(file, loc) {
            SignalLoadResult::Ok(signal) => signal,
            SignalLoadResult::Err { message } => return format!("export failed: {message}"),
        };
        let added = writer.add_group(&signal.times).and_then(|group| {
            group.add_channel_with_validity(
                &signal.name,
                &signal.unit,
                &signal.values,
                signal.valid.as_deref(),
            )
        });
        if let Err(e) = added {
            return format!("export failed: {e}");
        }
    }
    match writer.write_to_file(path) {
        Ok(()) => format!("exported {} channel(s) to {}", locs.len(), path.display()),
        Err(e) => format!("export failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_index_picks_the_closer_neighbor() {
        let times = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(nearest_index(&times, 1.4), 1);
        assert_eq!(nearest_index(&times, 1.6), 2);
        assert_eq!(nearest_index(&times, 1.0), 1);
    }

    #[test]
    fn nearest_index_clamps_outside_the_range() {
        let times = [5.0, 6.0, 7.0];
        assert_eq!(nearest_index(&times, -10.0), 0);
        assert_eq!(nearest_index(&times, 100.0), 2);
    }

    #[test]
    fn sample_at_checks_validity() {
        let signal = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 0,
            },
            name: "Voltage".to_string(),
            unit: "V".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0, 2.0, 3.0],
            values: vec![10.0, 20.0, 30.0, 40.0],
            valid: Some(vec![true, false, true, true]),
        };

        assert_eq!(sample_at(&signal, 0.1), (0, true));
        assert_eq!(sample_at(&signal, 0.9), (1, false));
        assert_eq!(sample_at(&signal, 2.0), (2, true));
    }
}
