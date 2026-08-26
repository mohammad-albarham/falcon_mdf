//! The GPS track panel: draw latitude against longitude over a shared time base.
//!
//! The coordinate pairing, interpolation, and validation rules live in
//! [`crate::gps`], which has no `Ui` in it and is tested directly.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use egui_plot::{Legend, Line, Plot, Points};

use crate::gps::{pair_gps, GpsRefusal, GpsSeries};
use crate::model::{ChannelRef, FileSlot, GpsAxes, OpenFiles, PlottedChannel};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};

/// One coordinate channel's decode state.
enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    Failed(String),
}

const CURSOR_A_COLOR: egui::Color32 = egui::Color32::from_rgb(0x33, 0x99, 0xff);
const CURSOR_B_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x99, 0x00);
const CURVE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x77, 0xb4);
const REFUSAL_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 80, 80);

pub struct GpsPanel {
    /// Decoded coordinate channels.
    slots: HashMap<ChannelRef, Slot>,
    /// The chosen axes (latitude and longitude), or `None` until selected.
    axes: Option<GpsAxes>,
    /// Whether sample points are drawn on top of the track line.
    show_points: bool,
}

impl Default for GpsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl GpsPanel {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            axes: None,
            show_points: false,
        }
    }

    pub fn reset(&mut self) {
        self.slots.clear();
        self.axes = None;
    }

    /// The chosen axes, for session persistence.
    pub fn axes(&self) -> Option<GpsAxes> {
        self.axes
    }

    /// Puts back axes a session remembered.
    pub fn set_axes(&mut self, axes: Option<GpsAxes>) {
        self.axes = axes;
        self.slots.clear();
    }

    /// Drops a chosen axis if its channel is no longer plotted.
    fn forget_missing_axes(&mut self, plotted: &[PlottedChannel]) {
        let present = |r: ChannelRef| plotted.iter().any(|p| p.is(r.file, r.loc));
        if self
            .axes
            .is_some_and(|a| !present(a.latitude) || !present(a.longitude))
        {
            self.axes = None;
        }
    }

    /// Starts decodes for the chosen coordinate channels.
    fn sync_slots(&mut self, ui: &egui::Ui, files: &OpenFiles) {
        let Some(axes) = self.axes else {
            self.slots.clear();
            return;
        };
        for axis in [axes.latitude, axes.longitude] {
            if self.slots.contains_key(&axis) {
                continue;
            }
            let Some(loaded) = files.get(axis.file) else {
                continue;
            };
            self.slots.insert(
                axis,
                Slot::Loading(spawn_signal_load(
                    Arc::clone(&loaded.file),
                    axis.loc,
                    ui.ctx().clone(),
                )),
            );
        }
        self.slots
            .retain(|r, _| *r == axes.latitude || *r == axes.longitude);
    }

    fn poll(&mut self) {
        for slot in self.slots.values_mut() {
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

    /// Coordinate channel pickers over the channels currently plotted.
    fn show_axis_pickers(
        &mut self,
        ui: &mut egui::Ui,
        files: &OpenFiles,
        plotted: &[PlottedChannel],
    ) {
        let two_files = files.has_second();
        let label_of = |p: &PlottedChannel| {
            if two_files {
                format!("{} \u{00b7} {}", p.file.label(), p.name)
            } else {
                p.name.clone()
            }
        };
        // Default to the first two plotted channels.
        if self.axes.is_none() && plotted.len() >= 2 {
            self.axes = Some(GpsAxes {
                latitude: ChannelRef::new(plotted[0].file, plotted[0].loc),
                longitude: ChannelRef::new(plotted[1].file, plotted[1].loc),
            });
        }
        let Some(mut axes) = self.axes else {
            return;
        };

        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for (axis_label, current) in
                [("Lat:", &mut axes.latitude), ("Lon:", &mut axes.longitude)]
            {
                ui.label(axis_label);
                let selected = plotted
                    .iter()
                    .find(|p| p.is(current.file, current.loc))
                    .map(&label_of)
                    .unwrap_or_else(|| "(pick a channel)".to_string());
                egui::ComboBox::from_id_salt(("gps_axis", axis_label))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for p in plotted {
                            let this = ChannelRef::new(p.file, p.loc);
                            if ui
                                .selectable_label(*current == this, label_of(p))
                                .clicked()
                            {
                                *current = this;
                                changed = true;
                            }
                        }
                    });
                ui.add_space(8.0);
            }
            ui.separator();
            ui.checkbox(&mut self.show_points, "Samples")
                .on_hover_text("Draw the paired samples on top of the track");
        });

        if changed {
            self.axes = Some(axes);
            self.slots
                .retain(|r, _| *r == axes.latitude || *r == axes.longitude);
        } else {
            self.axes = Some(axes);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        files: &OpenFiles,
        plotted: &[PlottedChannel],
        b_offset: f64,
        absolute_alignment: bool,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        self.forget_missing_axes(plotted);

        if plotted.len() < 2 {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("GPS needs two channels");
                ui.label(
                    "Plot at least two channels, then pick one for latitude and one for \
                     longitude to draw the track.",
                );
            });
            return;
        }

        self.show_axis_pickers(ui, files, plotted);
        self.sync_slots(ui, files);
        self.poll();
        ui.separator();

        let Some(axes) = self.axes else {
            return;
        };
        if axes.latitude == axes.longitude {
            ui.label(
                "Latitude and longitude are the same channel, which draws a straight diagonal \
                 and is not a GPS track. Pick different channels for latitude and longitude.",
            );
            return;
        }

        let mut signals = Vec::new();
        for (axis, r) in [("Latitude", axes.latitude), ("Longitude", axes.longitude)] {
            match self.slots.get(&r) {
                Some(Slot::Loaded(sig)) => signals.push(sig),
                Some(Slot::Failed(message)) => {
                    ui.colored_label(REFUSAL_COLOR, format!("{axis}: {message}"));
                    return;
                }
                Some(Slot::Loading(_)) | None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Decoding the {axis} channel\u{2026}"));
                    });
                    return;
                }
            }
        }
        let (lat_signal, lon_signal) = (signals[0], signals[1]);
        let offset_of = |file: FileSlot| match file {
            FileSlot::A => 0.0,
            FileSlot::B => b_offset,
        };

        let paired = pair_gps(
            lat_signal,
            offset_of(axes.latitude.file),
            lon_signal,
            offset_of(axes.longitude.file),
            axes.is_cross_file(),
            absolute_alignment,
        );

        let series = match paired {
            Ok(series) => series,
            Err(refusal) => {
                show_refusal(ui, &refusal);
                return;
            }
        };

        self.show_plot(ui, &series, lat_signal, lon_signal, cursor_a, cursor_b);
    }

    fn show_plot(
        &self,
        ui: &mut egui::Ui,
        series: &GpsSeries,
        lat_signal: &ChannelSignal,
        lon_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        let marker_a = cursor_a.and_then(|t| series.point_at(t));
        let marker_b = cursor_b.and_then(|t| series.point_at(t));

        let points = series.points.clone();
        let show_points = self.show_points || points.len() < 2;
        let name = format!("{} vs {}", lat_signal.name, lon_signal.name);

        Plot::new("gps_plot")
            .legend(Legend::default())
            .x_axis_label(axis_label(&lon_signal.name, &lon_signal.unit))
            .y_axis_label(axis_label(&lat_signal.name, &lat_signal.unit))
            .show(ui, |plot_ui| {
                if points.len() >= 2 {
                    plot_ui.line(
                        Line::new(name.clone(), points.clone())
                            .color(CURVE_COLOR)
                            .width(1.5),
                    );
                }
                if show_points {
                    plot_ui.points(
                        Points::new("samples", points)
                            .color(CURVE_COLOR)
                            .radius(2.0),
                    );
                }
                if let Some(m) = marker_a {
                    plot_ui.points(
                        Points::new("A", vec![m.point])
                            .color(CURSOR_A_COLOR)
                            .radius(5.0)
                            .shape(egui_plot::MarkerShape::Diamond),
                    );
                }
                if let Some(m) = marker_b {
                    plot_ui.points(
                        Points::new("B", vec![m.point])
                            .color(CURSOR_B_COLOR)
                            .radius(5.0)
                            .shape(egui_plot::MarkerShape::Diamond),
                    );
                }
            });

        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("{} points \u{00b7} ", series.points.len()));
            ui.weak(series.pairing.describe());
        });
        if series.dropped > 0 {
            ui.weak(format!(
                "{} paired sample(s) left out: invalid or NaN on one axis.",
                series.dropped
            ));
        }
        if series.points.len() == 1 {
            ui.weak("Only 1 paired sample \u{2014} a track needs at least 2 points.");
        } else if series.is_all_identical() {
            ui.weak("All coordinates are identical \u{2014} a track needs distinct points.");
        }

        self.show_cursor_readout(ui, series, lat_signal, lon_signal, cursor_a, cursor_b);
    }

    fn show_cursor_readout(
        &self,
        ui: &mut egui::Ui,
        series: &GpsSeries,
        lat_signal: &ChannelSignal,
        lon_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        if cursor_a.is_none() && cursor_b.is_none() {
            ui.weak(
                "Place the measurement cursors in the Plot tab to mark where the track was at \
                 an instant.",
            );
            return;
        }

        ui.separator();
        ui.strong("Measurement cursors");
        egui::Grid::new("gps_cursor_grid")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Cursor");
                ui.strong("Sample time");
                ui.label(axis_label(&lon_signal.name, &lon_signal.unit));
                ui.label(axis_label(&lat_signal.name, &lat_signal.unit));
                ui.end_row();

                for (label, color, cursor) in [
                    ("A", CURSOR_A_COLOR, cursor_a),
                    ("B", CURSOR_B_COLOR, cursor_b),
                ] {
                    ui.colored_label(color, label);
                    match cursor {
                        Some(t) => {
                            match series.point_at(t) {
                                Some(m) => {
                                    let drift = (m.time - t).abs();
                                    if drift > 1e-6 {
                                        ui.label(format!(
                                            "{:.6} s (cursor at {t:.6})",
                                            m.time
                                        ))
                                        .on_hover_text(
                                            "The nearest paired sample is this far from the \
                                             cursor: the track has a gap there.",
                                        );
                                    } else {
                                        ui.label(format!("{:.6} s", m.time));
                                    }
                                    ui.label(format!("{:.6}", m.longitude()));
                                    ui.label(format!("{:.6}", m.latitude()));
                                }
                                None => {
                                    let span = series.span();
                                    let detail = match span {
                                        Some((lo, hi)) => format!(
                                            "outside the paired span ({lo:.6}\u{2026}{hi:.6} s)"
                                        ),
                                        None => "outside the paired span".to_string(),
                                    };
                                    ui.weak(detail);
                                    ui.label("\u{2014}");
                                    ui.label("\u{2014}");
                                }
                            }
                        }
                        None => {
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

fn show_refusal(ui: &mut egui::Ui, refusal: &GpsRefusal) {
    ui.add_space(20.0);
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(REFUSAL_COLOR, "\u{26a0}");
        ui.colored_label(REFUSAL_COLOR, "These two channels cannot be drawn as a GPS track.");
    });
    ui.add_space(6.0);
    ui.label(refusal.message());
}

fn axis_label(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} [{unit}]")
    }
}
