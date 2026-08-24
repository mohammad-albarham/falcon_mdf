//! Numeric panel: shows every plotted channel's value at a single instant in time.
//!
//! Reuses `signal_loader::spawn_signal_load` to decode channels asynchronously
//! on worker threads without blocking the UI, caching decoded signals per
//! `ChannelLoc`.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use falcon_mdf::Mf4File;

use crate::model::{ChannelLoc, PlottedChannel};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};

/// Returns the timestamp and value `(timestamp, value)` of the sample at or
/// immediately before time `at`, skipping any invalid or NaN samples by looking
/// backwards.
///
/// Returns `None` if no valid sample exists at or before `at`, or if the series
/// is empty.
pub fn value_at(
    times: &[f64],
    values: &[f64],
    valid: Option<&[bool]>,
    at: f64,
) -> Option<(f64, f64)> {
    let len = times.len().min(values.len());
    if len == 0 {
        return None;
    }

    let idx = times[..len].partition_point(|&t| t <= at);
    if idx == 0 {
        return None;
    }

    let mut i = idx;
    while i > 0 {
        i -= 1;
        let is_valid = match valid {
            Some(v) => v.get(i).copied().unwrap_or(true),
            None => true,
        };
        if !is_valid {
            continue;
        }
        let val = values[i];
        if val.is_nan() {
            continue;
        }
        return Some((times[i], val));
    }

    None
}

enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    Failed(String),
}

/// The Numeric panel showing instantaneous channel values at a selected time.
pub struct NumericPanel {
    /// Instantaneous time coordinate for sample lookup.
    time: f64,
    /// Cached decoded signals or in-flight loading jobs keyed by channel location.
    slots: HashMap<ChannelLoc, Slot>,
}

impl Default for NumericPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl NumericPanel {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            slots: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
        self.slots.clear();
    }

    fn sync_slots(&mut self, ui: &egui::Ui, file: &Arc<Mf4File>, plotted: &[PlottedChannel]) {
        for channel in plotted {
            let loc = channel.loc;
            if self.slots.contains_key(&loc) {
                continue;
            }
            let slot = match channel_at(file, loc) {
                Some(ch) => match ch.unreadable() {
                    Some(reason) => Slot::Failed(format!("unreadable: {reason}")),
                    None => {
                        Slot::Loading(spawn_signal_load(Arc::clone(file), loc, ui.ctx().clone()))
                    }
                },
                None => Slot::Failed("channel not found in file".to_string()),
            };
            self.slots.insert(loc, slot);
        }
        self.slots
            .retain(|loc, _| plotted.iter().any(|p| p.loc == *loc));
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

    pub fn show(&mut self, ui: &mut egui::Ui, file: &Arc<Mf4File>, plotted: &[PlottedChannel]) {
        self.sync_slots(ui, file, plotted);
        self.poll();

        if plotted.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("No channels plotted. Select channels from the tree or channel list to display them here.");
            });
            return;
        }

        let mut min_time: Option<f64> = None;
        let mut max_time: Option<f64> = None;
        for slot in self.slots.values() {
            if let Slot::Loaded(sig) = slot {
                if let (Some(&first), Some(&last)) = (sig.times.first(), sig.times.last()) {
                    min_time = Some(min_time.map_or(first, |m| m.min(first)));
                    max_time = Some(max_time.map_or(last, |m| m.max(last)));
                }
            }
        }

        ui.horizontal(|ui| {
            ui.label("Instant:");
            ui.add(
                egui::DragValue::new(&mut self.time)
                    .speed(0.01)
                    .custom_formatter(|n, _| format!("{:.6}", n)),
            );

            if let Some(t_start) = min_time {
                if ui
                    .button("Start")
                    .on_hover_text(format!("Jump to start ({:.6})", t_start))
                    .clicked()
                {
                    self.time = t_start;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Start"));
            }

            if let Some(t_end) = max_time {
                if ui
                    .button("End")
                    .on_hover_text(format!("Jump to end ({:.6})", t_end))
                    .clicked()
                {
                    self.time = t_end;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("End"));
            }
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("numeric_panel_grid")
                    .num_columns(5)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Signal");
                        ui.strong("Unit");
                        ui.strong("Value");
                        ui.strong("Sample Time");
                        ui.strong("Status");
                        ui.end_row();

                        for channel in plotted {
                            let loc = channel.loc;
                            ui.horizontal(|ui| {
                                ui.colored_label(channel.color, "\u{25cf}");
                                ui.label(&channel.name);
                            });

                            match self.slots.get(&loc) {
                                Some(Slot::Loaded(sig)) => {
                                    ui.label(if sig.unit.is_empty() {
                                        "\u{2014}"
                                    } else {
                                        &sig.unit
                                    });

                                    match value_at(
                                        &sig.times,
                                        &sig.values,
                                        sig.valid.as_deref(),
                                        self.time,
                                    ) {
                                        Some((t_used, val)) => {
                                            ui.label(format!("{:.6}", val));
                                            ui.label(format!("{:.6} {}", t_used, sig.time_unit));

                                            let cand_idx =
                                                sig.times.partition_point(|&t| t <= self.time);
                                            if cand_idx > 0 && sig.times[cand_idx - 1] > t_used {
                                                ui.label(
                                                    egui::RichText::new(
                                                        "skipped invalid sample(s)",
                                                    )
                                                    .weak(),
                                                );
                                            } else if (t_used - self.time).abs() < 1e-9 {
                                                ui.label("exact match");
                                            } else {
                                                ui.label("held from earlier");
                                            }
                                        }
                                        None => {
                                            if sig.times.is_empty() {
                                                ui.label("(no samples)");
                                                ui.label("\u{2014}");
                                                ui.label("\u{2014}");
                                            } else if self.time < sig.times[0] {
                                                ui.label("(before first sample)");
                                                ui.label("\u{2014}");
                                                ui.label(format!(
                                                    "first at {:.6} {}",
                                                    sig.times[0], sig.time_unit
                                                ));
                                            } else {
                                                ui.label("(all prior samples invalid)");
                                                ui.label("\u{2014}");
                                                ui.label("\u{2014}");
                                            }
                                        }
                                    }
                                }
                                Some(Slot::Loading(_)) => {
                                    ui.label("\u{2014}");
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label("decoding\u{2026}");
                                    });
                                    ui.label("\u{2014}");
                                    ui.label("loading");
                                }
                                Some(Slot::Failed(err)) => {
                                    ui.label("\u{2014}");
                                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                                    ui.label("\u{2014}");
                                    ui.label("error");
                                }
                                None => {
                                    ui.label("\u{2014}");
                                    ui.label("waiting\u{2026}");
                                    ui.label("\u{2014}");
                                    ui.label("\u{2014}");
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
    }
}

fn channel_at(file: &Mf4File, loc: ChannelLoc) -> Option<&falcon_mdf::Channel> {
    file.data_groups()
        .get(loc.data_group_index)?
        .channel_groups
        .get(loc.channel_group_index)?
        .channels
        .get(loc.channel_index)
}
