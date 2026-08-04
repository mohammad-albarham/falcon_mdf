//! The plot panel: every plotted channel decimated against its master, in
//! overlay or stacked mode, with cursor readouts, zoom and pan. `egui_plot`
//! gives zoom/pan for free; this panel's job is feeding it decimated points
//! instead of raw samples (see `crate::decimate`), keeping one decode per
//! channel alive across frames, and surfacing failed decodes as text rather
//! than silence.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui_plot::{Legend, Line, Plot, VLine};
use falcon_mdf_gui::decimate::decimate_min_max_gaps;

use crate::model::{ChannelLoc, LoadedFile, PlottedChannel};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};

/// One plotted channel's decode state.
enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    /// Decode failed — or the channel declared itself unreadable before a
    /// decode was even attempted. Either way the message is shown in the
    /// plot area; a failed channel is never silently absent.
    Failed(String),
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

pub struct PlotPanel {
    slots: HashMap<ChannelLoc, Slot>,
    caches: HashMap<ChannelLoc, DecimationCache>,
    mode: PlotMode,
    /// The time under the cursor as of last frame, for stacked readouts:
    /// subplots are drawn top to bottom, so a subplot above the hovered one
    /// only learns the hovered time next frame. One frame of lag on a text
    /// label is invisible, and the drawn cursor itself is real-time.
    hovered_x: Option<f64>,
}

impl PlotPanel {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            caches: HashMap::new(),
            mode: PlotMode::Overlay,
            hovered_x: None,
        }
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
            let ch = &loaded.file.data_groups()[loc.data_group_index].channel_groups
                [loc.channel_group_index]
                .channels[loc.channel_index];
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
        // Removing a channel drops its slot. For a Loading slot that just
        // drops the receiver: the decode finishes on its worker thread and
        // the result is discarded.
        self.slots
            .retain(|loc, _| plotted.iter().any(|p| p.loc == *loc));
        self.caches
            .retain(|loc, _| plotted.iter().any(|p| p.loc == *loc));
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

    pub fn show(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile, plotted: &[PlottedChannel]) {
        self.sync_slots(ui, loaded, plotted);
        self.poll();

        if plotted.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("Click a channel in the list to plot it. Click again to remove it.");
            });
            return;
        }

        ui.horizontal(|ui| {
            ui.label("View:");
            ui.selectable_value(&mut self.mode, PlotMode::Overlay, "Overlay");
            ui.selectable_value(&mut self.mode, PlotMode::Stacked, "Stacked");
        });

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

        let any_loading = plotted
            .iter()
            .any(|p| p.visible && matches!(self.slots.get(&p.loc), Some(Slot::Loading(_))));
        if any_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Decoding channel\u{2026}");
            });
        }

        // The (channel, signal) pairs that can actually be drawn this frame.
        // A loaded channel with no samples is excluded here — there is no
        // line to draw — and reported below if nothing else remains.
        let drawable: Vec<(&PlottedChannel, &ChannelSignal)> = plotted
            .iter()
            .filter(|p| p.visible)
            .filter_map(|p| match self.slots.get(&p.loc) {
                Some(Slot::Loaded(signal)) if !signal.times.is_empty() => Some((p, signal)),
                _ => None,
            })
            .collect();

        if drawable.is_empty() {
            if !any_loading && plotted.iter().any(|p| p.visible) {
                ui.label("The plotted channels have no samples.");
            }
            return;
        }

        // The union of all visible time ranges: the shared X axis has to
        // cover every channel, including ones whose master starts later or
        // ends earlier than the first's.
        let full_range = drawable
            .iter()
            .map(|(_, s)| (s.times[0], *s.times.last().unwrap()))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (a, b)| {
                (lo.min(a), hi.max(b))
            });

        let caches = &mut self.caches;
        let hovered_x = &mut self.hovered_x;
        match self.mode {
            PlotMode::Overlay => Self::show_overlay(ui, caches, &drawable, full_range),
            PlotMode::Stacked => Self::show_stacked(ui, caches, hovered_x, &drawable, full_range),
        }
    }

    /// Associated functions rather than `&mut self` methods: `drawable`
    /// borrows `self.slots` at the call site, and these only need the other
    /// fields, so keeping them separate avoids borrowing all of `self`.
    fn show_overlay(
        ui: &mut egui::Ui,
        caches: &mut HashMap<ChannelLoc, DecimationCache>,
        drawable: &[(&PlottedChannel, &ChannelSignal)],
        full_range: (f64, f64),
    ) {
        let mut hovered_time = None;
        let first = drawable[0].1;

        Plot::new("overlay_plot")
            .legend(Legend::default())
            .x_axis_label(axis_label(&first.time_name, &first.time_unit))
            .show(ui, |plot_ui| {
                let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;
                let bounds = plot_ui.plot_bounds();
                for (channel, signal) in drawable {
                    // On a channel's first frame the plot still reports its
                    // default (0..1) bounds, so decimate against the full
                    // range until real bounds exist.
                    let x_range = if caches.contains_key(&signal.loc) {
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
                    let legend_name = axis_label(&signal.name, &signal.unit);
                    for segment in segments_for(caches, signal, x_range, n_columns) {
                        plot_ui.line(Line::new(legend_name.clone(), segment).color(channel.color));
                    }
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
            });

        match hovered_time {
            Some(t) => {
                for (channel, signal) in drawable {
                    ui.horizontal(|ui| {
                        ui.colored_label(channel.color, "\u{25cf}");
                        ui.label(readout(signal, t));
                    });
                }
            }
            None => {
                ui.label("Hover the plot for a value readout.");
            }
        }
    }

    fn show_stacked(
        ui: &mut egui::Ui,
        caches: &mut HashMap<ChannelLoc, DecimationCache>,
        hovered_x: &mut Option<f64>,
        drawable: &[(&PlottedChannel, &ChannelSignal)],
        full_range: (f64, f64),
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

        for (index, (channel, signal)) in drawable.iter().enumerate() {
            // The plot id is the channel's location, not its position in the
            // list, so adding or removing other channels neither collides
            // nor resets a subplot's remembered zoom.
            let mut plot = Plot::new((
                "stacked_plot",
                signal.loc.data_group_index,
                signal.loc.channel_group_index,
                signal.loc.channel_index,
            ))
            .link_axis("stacked_x", egui::Vec2b::new(true, false))
            .link_cursor("stacked_x", egui::Vec2b::new(true, false))
            .height(height)
            .include_x(full_range.0)
            .include_x(full_range.1)
            .y_axis_label(axis_label(&signal.name, &signal.unit));
            // Only the bottom subplot names the X axis; every subplot shares
            // it, and repeating the label just spends vertical space.
            if index == last {
                plot = plot.x_axis_label(axis_label(&signal.time_name, &signal.time_unit));
            }

            let response = plot.show(ui, |plot_ui| {
                let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;
                let bounds = plot_ui.plot_bounds();
                let x_range = if caches.contains_key(&signal.loc) {
                    (bounds.min()[0], bounds.max()[0])
                } else {
                    full_range
                };
                for segment in segments_for(caches, signal, x_range, n_columns) {
                    plot_ui.line(Line::new(signal.name.clone(), segment).color(channel.color));
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

            match hovered_now.or(*hovered_x) {
                Some(t) => {
                    ui.horizontal(|ui| {
                        ui.colored_label(channel.color, "\u{25cf}");
                        ui.label(readout(signal, t));
                    });
                }
                None => {
                    ui.label("Hover the plot for a value readout.");
                }
            }
        }
        *hovered_x = hovered_now;
    }
}

/// Decimated segments for one channel at the current view, from the cache
/// when the view hasn't moved since last frame.
fn segments_for(
    caches: &mut HashMap<ChannelLoc, DecimationCache>,
    signal: &ChannelSignal,
    x_range: (f64, f64),
    n_columns: usize,
) -> Vec<Vec<[f64; 2]>> {
    match caches.get(&signal.loc) {
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
                signal.loc,
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

/// The readout line for one signal at hovered time `t`. A sample the file
/// marks invalid is gapped out of the plot, so the readout must not quietly
/// show the garbage value the record held there either. Names the channel
/// and its unit, since several readouts are shown together once more than
/// one channel is plotted.
fn readout(signal: &ChannelSignal, t: f64) -> String {
    let i = nearest_index(&signal.times, t);
    let valid = match &signal.valid {
        Some(v) => v.get(i).copied().unwrap_or(true),
        None => true,
    };
    if valid {
        let value = axis_label(&format!("{:.6}", signal.values[i]), &signal.unit);
        format!(
            "{}: t = {:.6}    value = {}",
            signal.name, signal.times[i], value
        )
    } else {
        format!(
            "{}: t = {:.6}    (sample marked invalid)",
            signal.name, signal.times[i]
        )
    }
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
fn nearest_index(times: &[f64], t: f64) -> usize {
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
}
