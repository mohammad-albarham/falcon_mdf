//! The X-Y panel: one plotted channel against another, rather than against
//! time.
//!
//! The axes are picked from the channels already plotted, so the same tick in
//! the tree that puts a channel on the time plot makes it available here, and
//! the file badges and colours carry straight over. The pairing rules — and
//! every reason two channels might not be pairable at all — live in
//! [`crate::xy`], which has no `Ui` in it and is tested directly.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use egui_plot::{Legend, Line, Plot, Points};

use crate::model::{ChannelRef, FileSlot, OpenFiles, PlottedChannel, XyChannels};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};
use crate::xy::{pair_xy, XyRefusal, XySeries};

/// One axis channel's decode state. The same three-state shape the plot and
/// numeric panels use: a failed decode is shown as text, never as an empty
/// plot.
enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    Failed(String),
}

const CURSOR_A_COLOR: egui::Color32 = egui::Color32::from_rgb(0x33, 0x99, 0xff);
const CURSOR_B_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x99, 0x00);
const CURVE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x77, 0xb4);
const REFUSAL_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 80, 80);

pub struct XyPanel {
    /// Decoded axis channels, keyed the same way the plot panel keys its
    /// own: by file *and* location.
    slots: HashMap<ChannelRef, Slot>,
    /// The chosen axes, or `None` until two channels are plotted and picked.
    /// This is what the session stores.
    axes: Option<XyChannels>,
    /// Whether the sample points are drawn on top of the line. On a slow
    /// signal the line alone hides how the samples are spaced.
    show_points: bool,
}

impl Default for XyPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl XyPanel {
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

    /// The chosen axes, for the session.
    pub fn axes(&self) -> Option<XyChannels> {
        self.axes
    }

    /// Puts back axes a session remembered. The caller has already checked
    /// both locations against the files they name.
    pub fn set_axes(&mut self, axes: Option<XyChannels>) {
        self.axes = axes;
        self.slots.clear();
    }

    /// Drops a chosen axis whose channel is no longer available — the file it
    /// was in was closed, or it was unticked in the tree. Half an X-Y plot is
    /// not a plot.
    fn forget_missing_axes(&mut self, plotted: &[PlottedChannel]) {
        let present =
            |r: ChannelRef| plotted.iter().any(|p| p.is(r.file, r.loc));
        if self
            .axes
            .is_some_and(|a| !present(a.x) || !present(a.y))
        {
            self.axes = None;
        }
    }

    /// Starts decodes for the two chosen axes and drops everything else.
    fn sync_slots(&mut self, ui: &egui::Ui, files: &OpenFiles) {
        let Some(axes) = self.axes else {
            self.slots.clear();
            return;
        };
        for axis in [axes.x, axes.y] {
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
            .retain(|r, _| *r == axes.x || *r == axes.y);
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

    /// The two axis pickers, over the channels currently plotted.
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
        // Default to the first two plotted channels, so opening the tab with
        // channels already plotted shows a curve rather than two empty boxes.
        if self.axes.is_none() && plotted.len() >= 2 {
            self.axes = Some(XyChannels {
                x: ChannelRef::new(plotted[0].file, plotted[0].loc),
                y: ChannelRef::new(plotted[1].file, plotted[1].loc),
            });
        }
        let Some(mut axes) = self.axes else {
            return;
        };

        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for (axis_label, current) in
                [("X:", &mut axes.x), ("Y:", &mut axes.y)]
            {
                ui.label(axis_label);
                let selected = plotted
                    .iter()
                    .find(|p| p.is(current.file, current.loc))
                    .map(&label_of)
                    .unwrap_or_else(|| "(pick a channel)".to_string());
                egui::ComboBox::from_id_salt(("xy_axis", axis_label))
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
                .on_hover_text("Draw the paired samples on top of the line");
        });

        if changed {
            self.axes = Some(axes);
            self.slots.retain(|r, _| *r == axes.x || *r == axes.y);
        } else {
            self.axes = Some(axes);
        }
    }

    /// `b_offset` is the shift the plot panel applies to the second file, and
    /// `absolute_alignment` whether that shift comes from the two headers'
    /// wall clock. Both are passed in rather than recomputed so the X-Y view
    /// and the time plot can never disagree about where file B sits.
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
                ui.heading("X-Y needs two channels");
                ui.label(
                    "Plot at least two channels, then pick one for each axis here to see how \
                     they move together.",
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
        if axes.x == axes.y {
            ui.label(
                "X and Y are the same channel, which draws a straight line and says nothing. \
                 Pick a different channel for one of the axes.",
            );
            return;
        }

        // A decode that failed or has not landed yet is said out loud; the
        // pairing below cannot start without both signals.
        let mut signals = Vec::new();
        for (axis, r) in [("X", axes.x), ("Y", axes.y)] {
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
        let (x_signal, y_signal) = (signals[0], signals[1]);
        let offset_of = |file: FileSlot| match file {
            FileSlot::A => 0.0,
            FileSlot::B => b_offset,
        };

        let paired = pair_xy(
            x_signal,
            offset_of(axes.x.file),
            y_signal,
            offset_of(axes.y.file),
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

        self.show_plot(ui, &series, x_signal, y_signal, cursor_a, cursor_b);
    }

    fn show_plot(
        &self,
        ui: &mut egui::Ui,
        series: &XySeries,
        x_signal: &ChannelSignal,
        y_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        let marker_a = cursor_a.and_then(|t| series.point_at(t));
        let marker_b = cursor_b.and_then(|t| series.point_at(t));

        let points = series.points.clone();
        let show_points = self.show_points;
        let name = format!("{} vs {}", y_signal.name, x_signal.name);

        Plot::new("xy_plot")
            .legend(Legend::default())
            .x_axis_label(axis_label(&x_signal.name, &x_signal.unit))
            .y_axis_label(axis_label(&y_signal.name, &y_signal.unit))
            // Equal scaling is wrong here: the two axes are different
            // quantities in different units, so a "square" aspect would mean
            // nothing and would waste most of the panel.
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new(name.clone(), points.clone())
                        .color(CURVE_COLOR)
                        .width(1.5),
                );
                if show_points {
                    plot_ui.points(
                        Points::new("samples", points)
                            .color(CURVE_COLOR)
                            .radius(2.0),
                    );
                }
                // The cursors are times, and this plot has no time axis, so
                // they appear as the point the curve was at that instant.
                if let Some(p) = marker_a {
                    plot_ui.points(
                        Points::new("A", vec![p])
                            .color(CURSOR_A_COLOR)
                            .radius(5.0)
                            .shape(egui_plot::MarkerShape::Diamond),
                    );
                }
                if let Some(p) = marker_b {
                    plot_ui.points(
                        Points::new("B", vec![p])
                            .color(CURSOR_B_COLOR)
                            .radius(5.0)
                            .shape(egui_plot::MarkerShape::Diamond),
                    );
                }
            });

        // How the curve was built, always, under the plot: an X-Y curve gives
        // the reader no way to tell an exact pairing from an interpolated one.
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

        self.show_cursor_readout(ui, series, x_signal, y_signal, cursor_a, cursor_b);
    }

    fn show_cursor_readout(
        &self,
        ui: &mut egui::Ui,
        series: &XySeries,
        x_signal: &ChannelSignal,
        y_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        if cursor_a.is_none() && cursor_b.is_none() {
            ui.weak(
                "Place the measurement cursors in the Plot tab to mark where the curve was at \
                 an instant.",
            );
            return;
        }

        ui.separator();
        ui.strong("Measurement cursors");
        egui::Grid::new("xy_cursor_grid")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Cursor");
                ui.strong("Time");
                ui.label(axis_label(&x_signal.name, &x_signal.unit));
                ui.label(axis_label(&y_signal.name, &y_signal.unit));
                ui.end_row();

                for (label, color, cursor) in [
                    ("A", CURSOR_A_COLOR, cursor_a),
                    ("B", CURSOR_B_COLOR, cursor_b),
                ] {
                    ui.colored_label(color, label);
                    match cursor {
                        Some(t) => {
                            ui.label(format!("{t:.6} s"));
                            match series.point_at(t) {
                                Some(p) => {
                                    ui.label(format!("{:.6}", p[0]));
                                    ui.label(format!("{:.6}", p[1]));
                                }
                                None => {
                                    // Said rather than shown as the nearest
                                    // end of the curve, which would read as
                                    // the curve being there at that time.
                                    let span = series.span();
                                    let detail = match span {
                                        Some((lo, hi)) => format!(
                                            "outside the paired span ({lo:.6}\u{2026}{hi:.6} s)"
                                        ),
                                        None => "outside the paired span".to_string(),
                                    };
                                    ui.weak(detail);
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

/// A refusal, in place of the plot. Loud enough to read as a decision the
/// viewer made, not as a blank panel.
fn show_refusal(ui: &mut egui::Ui, refusal: &XyRefusal) {
    ui.add_space(20.0);
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(REFUSAL_COLOR, "\u{26a0}");
        ui.colored_label(REFUSAL_COLOR, "These two channels cannot be put on a common time base.");
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
