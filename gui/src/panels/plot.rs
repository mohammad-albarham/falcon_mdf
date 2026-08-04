//! The plot panel: the selected channel decimated against its master, with
//! a cursor readout, zoom and pan. `egui_plot` gives zoom/pan for free; this
//! panel's job is feeding it decimated points instead of raw samples (see
//! `crate::decimate`) and showing a spinner while a large channel decodes.

use std::sync::mpsc::{Receiver, TryRecvError};

use egui_plot::{Line, Plot, VLine};
use falcon_mdf_gui::decimate::decimate_min_max;

use crate::model::{ChannelLoc, LoadedFile};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};

enum State {
    Idle,
    Loading {
        loc: ChannelLoc,
        rx: Receiver<SignalLoadResult>,
    },
    Loaded(ChannelSignal),
    Failed {
        loc: ChannelLoc,
        message: String,
    },
}

/// The last decimation computed, so a frame where the view hasn't moved
/// doesn't re-scan the signal. Keyed on the channel, the visible time range
/// and the pixel width `egui_plot` reported: any change to any of those means
/// different pixel columns, so the cache is stale.
struct DecimationCache {
    loc: ChannelLoc,
    x_range: (f64, f64),
    n_columns: usize,
    points: Vec<[f64; 2]>,
}

pub struct PlotPanel {
    state: State,
    cache: Option<DecimationCache>,
}

impl PlotPanel {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            cache: None,
        }
    }

    fn current_loc(&self) -> Option<ChannelLoc> {
        match &self.state {
            State::Idle => None,
            State::Loading { loc, .. } | State::Failed { loc, .. } => Some(*loc),
            State::Loaded(sig) => Some(sig.loc),
        }
    }

    fn poll(&mut self) {
        let State::Loading { rx, .. } = &self.state else {
            return;
        };
        match rx.try_recv() {
            Ok(SignalLoadResult::Ok(sig)) => {
                self.cache = None;
                self.state = State::Loaded(sig);
            }
            Ok(SignalLoadResult::Err { loc, message }) => {
                self.state = State::Failed { loc, message };
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if let State::Loading { loc, .. } = &self.state {
                    self.state = State::Failed {
                        loc: *loc,
                        message: "signal loader thread ended without a result".to_string(),
                    };
                }
            }
        }
    }

    /// Shows the plot for `selected`, starting (or restarting) a decode when
    /// it differs from whatever this panel last loaded.
    pub fn show(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile, selected: ChannelLoc) {
        if self.current_loc() != Some(selected) {
            let rx = spawn_signal_load(loaded.file.clone(), selected, ui.ctx().clone());
            self.state = State::Loading { loc: selected, rx };
        }
        self.poll();

        match &self.state {
            State::Idle => {}
            State::Loading { .. } => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.spinner();
                    ui.label("Decoding channel\u{2026}");
                });
            }
            State::Failed { message, .. } => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading("Could not read this channel");
                    ui.add_space(10.0);
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), message);
                });
            }
            State::Loaded(signal) => Self::show_plot(ui, &mut self.cache, signal),
        }
    }

    /// An associated function rather than a `&mut self` method: `signal` is
    /// borrowed from `self.state` at the call site, and this only needs
    /// `self.cache`, so keeping it separate avoids borrowing all of `self`.
    fn show_plot(ui: &mut egui::Ui, cache: &mut Option<DecimationCache>, signal: &ChannelSignal) {
        if signal.times.is_empty() {
            ui.label("This channel has no samples.");
            return;
        }

        let full_range = (signal.times[0], *signal.times.last().unwrap());
        let loc = signal.loc;
        let mut hovered_time = None;

        let x_label = axis_label(&signal.time_name, &signal.time_unit);
        let y_label = axis_label(&signal.name, &signal.unit);

        Plot::new((
            "channel_plot",
            loc.data_group_index,
            loc.channel_group_index,
            loc.channel_index,
        ))
        .x_axis_label(x_label)
        .y_axis_label(y_label)
        .show(ui, |plot_ui| {
            let n_columns = plot_ui.response().rect.width().round().max(1.0) as usize;

            let x_range = match cache {
                Some(c) if c.loc == loc => {
                    let bounds = plot_ui.plot_bounds();
                    (bounds.min()[0], bounds.max()[0])
                }
                _ => full_range,
            };

            let points = match cache {
                Some(c) if c.loc == loc && c.x_range == x_range && c.n_columns == n_columns => {
                    c.points.clone()
                }
                _ => {
                    let points =
                        decimate_min_max(&signal.times, &signal.values, x_range, n_columns);
                    *cache = Some(DecimationCache {
                        loc,
                        x_range,
                        n_columns,
                        points: points.clone(),
                    });
                    points
                }
            };

            plot_ui.line(Line::new(signal.name.clone(), points));

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
                let i = nearest_index(&signal.times, t);
                ui.label(format!(
                    "t = {:.6}    value = {:.6}",
                    signal.times[i], signal.values[i]
                ));
            }
            None => {
                ui.label("Hover the plot for a value readout.");
            }
        }
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
