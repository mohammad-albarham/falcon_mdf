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

use egui_plot::{Legend, Line, Plot, PlotPoint, PlotPoints, Points};

use crate::decimate::decimate_curve;

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

/// The paired curve for the current axes, kept across frames.
///
/// Pairing is an O(n) walk with resampling allocations in it, and nothing
/// about it changes between two repaints of the same view — so it runs when
/// an axis, the file-B offset or the alignment mode changes, not per frame.
/// What is drawn is decimated against the view (`drawn`, rebuilt only when
/// the view moves or resizes), so `egui_plot` never tessellates a
/// million-point curve frame after frame. The series itself stays untouched
/// for the cursor lookups, which answer from the undecimated curve.
struct XyDrawCache {
    axes: XyChannels,
    b_offset: f64,
    absolute_alignment: bool,
    series: XySeries,
    /// The x extent of the whole curve, for the first frame: the plot's own
    /// bounds are not meaningful until something has been drawn.
    full_x_range: (f64, f64),
    /// The view the drawn points were decimated for. `(NaN, NaN)` until the
    /// first draw.
    view: (f64, f64),
    n_columns: usize,
    drawn: Arc<Vec<PlotPoint>>,
}

impl XyDrawCache {
    /// The points to draw for `x_range` in `n_columns` pixel columns,
    /// re-decimating only when the view moved.
    fn points_for(&mut self, x_range: (f64, f64), n_columns: usize) -> Arc<Vec<PlotPoint>> {
        if self.view != x_range || self.n_columns != n_columns {
            self.view = x_range;
            self.n_columns = n_columns;
            self.drawn = Arc::new(
                decimate_curve(&self.series.points, x_range, n_columns)
                    .into_iter()
                    .map(|p| PlotPoint::new(p[0], p[1]))
                    .collect(),
            );
        }
        Arc::clone(&self.drawn)
    }
}

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
    /// The last pairing drawn, rebuilt only when its inputs change.
    cache: Option<XyDrawCache>,
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
            cache: None,
        }
    }

    pub fn reset(&mut self) {
        self.slots.clear();
        self.axes = None;
        self.cache = None;
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
        self.cache = None;
    }

    /// Drops a chosen axis whose channel is no longer available — the file it
    /// was in was closed, or it was unticked in the tree. Half an X-Y plot is
    /// not a plot.
    fn forget_missing_axes(&mut self, plotted: &[PlottedChannel]) {
        let present = |r: ChannelRef| plotted.iter().any(|p| p.is(r.file, r.loc));
        if self.axes.is_some_and(|a| !present(a.x) || !present(a.y)) {
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
        self.slots.retain(|r, _| *r == axes.x || *r == axes.y);
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
            for (axis_label, current) in [("X:", &mut axes.x), ("Y:", &mut axes.y)] {
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
                            if ui.selectable_label(*current == this, label_of(p)).clicked() {
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

        // Re-pair only when a pairing input changed; an unchanged view
        // reuses the cached curve and its converted points.
        let stale = self.cache.as_ref().is_none_or(|c| {
            c.axes != axes || c.b_offset != b_offset || c.absolute_alignment != absolute_alignment
        });
        if stale {
            let paired = pair_xy(
                x_signal,
                offset_of(axes.x.file),
                y_signal,
                offset_of(axes.y.file),
                axes.is_cross_file(),
                absolute_alignment,
            );
            match paired {
                Ok(series) => {
                    let full_x_range = series
                        .points
                        .iter()
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                            (lo.min(p[0]), hi.max(p[0]))
                        });
                    self.cache = Some(XyDrawCache {
                        axes,
                        b_offset,
                        absolute_alignment,
                        series,
                        full_x_range,
                        view: (f64::NAN, f64::NAN),
                        n_columns: 0,
                        drawn: Arc::new(Vec::new()),
                    });
                }
                Err(refusal) => {
                    self.cache = None;
                    show_refusal(ui, &refusal);
                    return;
                }
            }
        }
        let cache = self.cache.as_mut().expect("just built");

        Self::show_plot(
            self.show_points,
            ui,
            cache,
            x_signal,
            y_signal,
            cursor_a,
            cursor_b,
        );
    }

    fn show_plot(
        show_samples: bool,
        ui: &mut egui::Ui,
        cache: &mut XyDrawCache,
        x_signal: &ChannelSignal,
        y_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        // Computed and dropped before the plot closure, so the closure is
        // free to re-decimate the cache for a moved view.
        let (marker_a, marker_b) = {
            let series = &cache.series;
            (
                cursor_a.and_then(|t| series.point_at(t)),
                cursor_b.and_then(|t| series.point_at(t)),
            )
        };
        // The decimated points live here, outside the closure: a borrowed
        // slice has to outlive `Plot::show`, which keeps what it was given
        // until tessellation.
        let mut drawn_store: Vec<Arc<Vec<PlotPoint>>> = Vec::new();

        // `Line` needs two points to draw anything, so a single paired sample
        // would leave a blank canvas under a caption saying "1 points". The
        // markers go on regardless of the checkbox in that case. (R3
        // finding 5.1.)
        let name = format!("{} vs {}", y_signal.name, x_signal.name);
        let first_frame = cache.n_columns == 0;

        Plot::new("xy_plot")
            .legend(Legend::default())
            .x_axis_label(axis_label(&x_signal.name, &x_signal.unit))
            .y_axis_label(axis_label(&y_signal.name, &y_signal.unit))
            // Equal scaling is wrong here: the two axes are different
            // quantities in different units, so a "square" aspect would mean
            // nothing and would waste most of the panel.
            .show(ui, |plot_ui| {
                let bounds = plot_ui.plot_bounds();
                // The first frame's bounds predate any data being drawn, so
                // decimate against the whole curve once; from the second
                // frame on the view the user actually sees drives it.
                let x_range = if first_frame {
                    cache.full_x_range
                } else {
                    (bounds.min()[0], bounds.max()[0])
                };
                let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;
                drawn_store.push(cache.points_for(x_range, n_columns));
                let points = drawn_store.last().expect("just pushed").as_slice();
                let show_points = show_samples || points.len() < 2;
                plot_ui.line(
                    Line::new(name.clone(), PlotPoints::Borrowed(points))
                        .color(CURVE_COLOR)
                        .width(1.5),
                );
                if show_points {
                    plot_ui.points(
                        Points::new("samples", PlotPoints::Borrowed(points))
                            .color(CURVE_COLOR)
                            .radius(2.0),
                    );
                }
                // The cursors are times, and this plot has no time axis, so
                // they appear as the point the curve was at that instant.
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

        // How the curve was built, always, under the plot: an X-Y curve gives
        // the reader no way to tell an exact pairing from an interpolated one.
        // The count is the pairing's, not the drawn points' — decimation is a
        // rendering decision and must not restate the measurement.
        let series = &cache.series;
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

        Self::show_cursor_readout(ui, &cache.series, x_signal, y_signal, cursor_a, cursor_b);
    }

    fn show_cursor_readout(
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
                ui.strong("Sample time");
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
                            match series.point_at(t) {
                                Some(m) => {
                                    // The sample's own instant, not the one
                                    // the cursor was dropped at: across a gap
                                    // in the recording those differ, and the
                                    // cursor's time would be a claim the data
                                    // does not support.
                                    let drift = (m.time - t).abs();
                                    if drift > 1e-6 {
                                        ui.label(format!("{:.6} s (cursor at {t:.6})", m.time))
                                            .on_hover_text(
                                                "The nearest paired sample is this far from the \
                                             cursor: the curve has a gap there.",
                                            );
                                    } else {
                                        ui.label(format!("{:.6} s", m.time));
                                    }
                                    ui.label(format!("{:.6}", m.point[0]));
                                    ui.label(format!("{:.6}", m.point[1]));
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
        ui.colored_label(
            REFUSAL_COLOR,
            "These two channels cannot be put on a common time base.",
        );
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
