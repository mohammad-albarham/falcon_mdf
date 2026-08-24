//! Per-channel statistics: count, range, mean, spread, and a distribution.
//!
//! Computed off the UI thread (same pattern as `signal_loader.rs` and
//! `plot.rs`): `Signal` is `Send + Sync` and owns its data, so decoding and
//! statistics calculation happen on a worker thread, and results are cached
//! per `ChannelLoc`.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use falcon_mdf::Mf4File;

use crate::model::ChannelLoc;

/// Number of bins for the value distribution histogram.
const HISTOGRAM_BIN_COUNT: usize = 50;

/// Time axis summary derived from the group's master channel.
#[derive(Clone, Debug)]
struct TimeAxisStats {
    first_timestamp: f64,
    last_timestamp: f64,
    duration: f64,
    time_unit: String,
    sample_rate: Option<f64>,
}

/// Precomputed histogram bin data.
#[derive(Clone, Debug)]
struct HistogramData {
    /// `(center, count, width)` for each bar.
    bars: Vec<(f64, f64, f64)>,
}

/// Statistics calculated for a single channel.
#[derive(Clone, Debug)]
struct ChannelStats {
    channel_name: String,
    channel_unit: String,
    /// What the figures are counted over, when that is not one number per
    /// sample. An array channel decodes to every element of every sample, so
    /// "124 samples" would be a different number from the one below and the
    /// difference has to be said out loud rather than left to be noticed.
    counted_over: Option<String>,
    sample_count: usize,
    valid_count: usize,
    excluded_count: usize,
    nan_inf_count: usize,
    min: Option<f64>,
    max: Option<f64>,
    peak_to_peak: Option<f64>,
    mean: Option<f64>,
    std_dev: Option<f64>,
    median: Option<f64>,
    /// The 5th, 25th, 75th and 95th percentiles, in that order. They say
    /// where the bulk of a channel sat in a way the mean and the extremes
    /// cannot: a spike moves the maximum but not the 95th.
    percentiles: Option<[f64; 4]>,
    first_value: Option<f64>,
    last_value: Option<f64>,
    time_axis: Option<TimeAxisStats>,
    histogram: Option<HistogramData>,
}

enum StatsResult {
    Ok(Box<ChannelStats>),
    Err(String),
}

/// The computation state for a single channel location.
enum StatsSlot {
    Loading(Receiver<StatsResult>),
    Ready(Box<ChannelStats>),
    NoSamples,
    Unreadable(String),
    Failed(String),
}

/// Statistics for the selected channel, computed off the UI thread.
pub struct StatsPanel {
    slots: HashMap<ChannelLoc, StatsSlot>,
}

impl Default for StatsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsPanel {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    /// Drops every cached result. Called when a new file is opened.
    pub fn reset(&mut self) {
        self.slots.clear();
    }

    /// Draws statistics for `loc`, starting the computation if needed.
    pub fn show(&mut self, ui: &mut egui::Ui, file: &Arc<Mf4File>, loc: ChannelLoc) {
        self.ensure_slot(ui, file, loc);
        self.poll(loc);

        match self.slots.get(&loc) {
            Some(StatsSlot::Loading(_)) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Computing channel statistics\u{2026}");
                });
            }
            Some(StatsSlot::NoSamples) => {
                ui.label("This channel has no samples.");
            }
            Some(StatsSlot::Unreadable(reason)) => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Unreadable channel: {reason}"),
                );
            }
            Some(StatsSlot::Failed(err)) => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Failed to decode channel: {err}"),
                );
            }
            Some(StatsSlot::Ready(stats)) => {
                Self::show_stats_grid(ui, stats);
                Self::show_histogram(ui, stats, loc);
            }
            None => {}
        }
    }

    fn ensure_slot(&mut self, ui: &egui::Ui, file: &Arc<Mf4File>, loc: ChannelLoc) {
        if self.slots.contains_key(&loc) {
            return;
        }

        let channel_lookup = file
            .data_groups()
            .get(loc.data_group_index)
            .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
            .and_then(|cg| {
                let sample_count = cg.sample_count;
                cg.channels
                    .get(loc.channel_index)
                    .map(|ch| (ch, sample_count))
            });

        let slot = match channel_lookup {
            None => StatsSlot::Failed("channel not found in file".to_string()),
            Some((channel, _)) if channel.unreadable().is_some() => {
                StatsSlot::Unreadable(channel.unreadable().unwrap().to_string())
            }
            Some((_, 0)) => StatsSlot::NoSamples,
            Some(_) => {
                let rx = spawn_compute_stats(Arc::clone(file), loc, ui.ctx().clone());
                StatsSlot::Loading(rx)
            }
        };

        self.slots.insert(loc, slot);
    }

    fn poll(&mut self, loc: ChannelLoc) {
        let result = match self.slots.get_mut(&loc) {
            Some(StatsSlot::Loading(rx)) => rx.try_recv(),
            _ => return,
        };

        match result {
            Ok(StatsResult::Ok(stats)) => {
                self.slots.insert(loc, StatsSlot::Ready(stats));
            }
            Ok(StatsResult::Err(message)) => {
                self.slots.insert(loc, StatsSlot::Failed(message));
            }
            Err(TryRecvError::Disconnected) => {
                self.slots.insert(
                    loc,
                    StatsSlot::Failed("worker thread ended without a result".to_string()),
                );
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn show_stats_grid(ui: &mut egui::Ui, stats: &ChannelStats) {
        // The numbers below mean nothing without the channel they are about:
        // the tab can be reached with any channel selected, and the tree that
        // says which one may be scrolled somewhere else entirely.
        ui.heading(if stats.channel_unit.is_empty() {
            stats.channel_name.clone()
        } else {
            format!("{} [{}]", stats.channel_name, stats.channel_unit)
        });
        if let Some(note) = &stats.counted_over {
            ui.weak(note);
        }
        egui::Grid::new("channel_statistics_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label(if stats.counted_over.is_some() {
                    "Value count"
                } else {
                    "Sample count"
                });
                ui.label(stats.sample_count.to_string());
                ui.end_row();

                ui.label("Valid count");
                ui.label(stats.valid_count.to_string());
                ui.end_row();

                ui.label("Excluded (invalid)");
                ui.label(stats.excluded_count.to_string());
                ui.end_row();

                ui.label("Min");
                ui.label(format_opt_value(stats.min, &stats.channel_unit));
                ui.end_row();

                ui.label("Max");
                ui.label(format_opt_value(stats.max, &stats.channel_unit));
                ui.end_row();

                ui.label("Peak-to-peak");
                ui.label(format_opt_value(stats.peak_to_peak, &stats.channel_unit));
                ui.end_row();

                ui.label("Mean");
                ui.label(format_opt_value(stats.mean, &stats.channel_unit));
                ui.end_row();

                ui.label("Standard deviation");
                ui.label(format_opt_value(stats.std_dev, &stats.channel_unit));
                ui.end_row();

                // Ordered as a distribution reads, with the median in its
                // place among the rest rather than listed apart from them.
                if let Some([p5, p25, p75, p95]) = stats.percentiles {
                    ui.label("5th percentile");
                    ui.label(format_value_with_unit(p5, &stats.channel_unit));
                    ui.end_row();

                    ui.label("25th percentile");
                    ui.label(format_value_with_unit(p25, &stats.channel_unit));
                    ui.end_row();

                    ui.label("Median");
                    ui.label(format_opt_value(stats.median, &stats.channel_unit));
                    ui.end_row();

                    ui.label("75th percentile");
                    ui.label(format_value_with_unit(p75, &stats.channel_unit));
                    ui.end_row();

                    ui.label("95th percentile");
                    ui.label(format_value_with_unit(p95, &stats.channel_unit));
                    ui.end_row();
                } else {
                    ui.label("Median");
                    ui.label(format_opt_value(stats.median, &stats.channel_unit));
                    ui.end_row();
                }

                ui.label("First value");
                ui.label(format_opt_value(stats.first_value, &stats.channel_unit));
                ui.end_row();

                ui.label("Last value");
                ui.label(format_opt_value(stats.last_value, &stats.channel_unit));
                ui.end_row();

                ui.label("NaN / infinite count");
                ui.label(stats.nan_inf_count.to_string());
                ui.end_row();

                match &stats.time_axis {
                    Some(time) => {
                        ui.label("First timestamp");
                        ui.label(format_value_with_unit(
                            time.first_timestamp,
                            &time.time_unit,
                        ));
                        ui.end_row();

                        ui.label("Last timestamp");
                        ui.label(format_value_with_unit(time.last_timestamp, &time.time_unit));
                        ui.end_row();

                        ui.label("Duration");
                        // Said in hours and minutes when it is long enough
                        // for the raw seconds to stop being readable; the
                        // exact figure stays beside it.
                        if time.time_unit == "s" {
                            ui.label(format!(
                                "{} ({})",
                                crate::format::duration(time.duration),
                                format_value_with_unit(time.duration, &time.time_unit)
                            ));
                        } else {
                            ui.label(format_value_with_unit(time.duration, &time.time_unit));
                        }
                        ui.end_row();

                        ui.label("Mean sample rate");
                        match time.sample_rate {
                            Some(rate) => {
                                ui.label(format!("{:.4} samples/s", rate));
                            }
                            None => {
                                ui.label("\u{2014}");
                            }
                        }
                        ui.end_row();
                    }
                    None => {
                        ui.label("Time axis");
                        ui.label("no master channel");
                        ui.end_row();
                    }
                }
            });
    }

    fn show_histogram(ui: &mut egui::Ui, stats: &ChannelStats, loc: ChannelLoc) {
        ui.separator();
        ui.strong("Distribution");

        if stats.valid_count == 0 {
            ui.label("No valid samples to display histogram.");
            return;
        }

        if let (Some(min), Some(max)) = (stats.min, stats.max) {
            if min == max {
                ui.label(format!(
                    "All valid samples have the same value ({}).",
                    format_value_with_unit(min, &stats.channel_unit)
                ));
                return;
            }
        }

        let Some(hist) = &stats.histogram else {
            ui.label("No valid samples to display histogram.");
            return;
        };

        let bars: Vec<egui_plot::Bar> = hist
            .bars
            .iter()
            .map(|&(center, count, width)| egui_plot::Bar::new(center, count).width(width))
            .collect();

        let chart_name = if stats.channel_unit.is_empty() {
            stats.channel_name.clone()
        } else {
            format!("{} [{}]", stats.channel_name, stats.channel_unit)
        };

        let chart = egui_plot::BarChart::new(chart_name, bars)
            .color(egui::Color32::from_rgb(0x1f, 0x77, 0xb4));

        let x_label = if stats.channel_unit.is_empty() {
            "Value".to_string()
        } else {
            format!("Value [{}]", stats.channel_unit)
        };

        egui_plot::Plot::new((
            "stats_histogram",
            loc.data_group_index,
            loc.channel_group_index,
            loc.channel_index,
        ))
        .height(180.0)
        .x_axis_label(x_label)
        .y_axis_label("Count")
        .show(ui, |plot_ui| {
            plot_ui.bar_chart(chart);
        });
    }
}

/// Decodes the channel and master channel off the UI thread and calculates summary statistics.
fn spawn_compute_stats(
    file: Arc<Mf4File>,
    loc: ChannelLoc,
    ctx: egui::Context,
) -> Receiver<StatsResult> {
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        let result = compute_stats(&file, loc);
        let _ = tx.send(result);
        ctx.request_repaint();
    });

    rx
}

fn compute_stats(file: &Mf4File, loc: ChannelLoc) -> StatsResult {
    let channel = match file
        .data_groups()
        .get(loc.data_group_index)
        .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
        .and_then(|cg| cg.channels.get(loc.channel_index))
    {
        Some(ch) => ch,
        None => return StatsResult::Err("channel not found in file".to_string()),
    };

    let signal = match file.signal(channel) {
        Ok(s) => s,
        Err(e) => return StatsResult::Err(e.to_string()),
    };

    let values = match signal.values_f64() {
        Ok(v) => v,
        Err(e) => return StatsResult::Err(e.to_string()),
    };

    let validity = signal.validity();
    let sample_count = values.len();

    let mut valid_count = 0usize;
    let mut excluded_count = 0usize;
    let mut nan_inf_count = 0usize;
    let mut first_value = None;
    let mut last_value = None;
    let mut finite_valid_values = Vec::new();

    for (i, &val) in values.iter().enumerate() {
        let is_valid = match &validity {
            Some(v) => v.get(i).copied().unwrap_or(true),
            None => true,
        };

        if !is_valid {
            excluded_count += 1;
            continue;
        }

        valid_count += 1;
        if first_value.is_none() {
            first_value = Some(val);
        }
        last_value = Some(val);

        if val.is_finite() {
            finite_valid_values.push(val);
        } else {
            nan_inf_count += 1;
        }
    }

    let (min, max, peak_to_peak, mean, std_dev, median, percentiles, histogram) =
        if finite_valid_values.is_empty() {
            (None, None, None, None, None, None, None, None)
        } else {
            let min = finite_valid_values
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let max = finite_valid_values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let peak_to_peak = max - min;

            let count_f = finite_valid_values.len() as f64;
            let sum: f64 = finite_valid_values.iter().sum();
            let mean = sum / count_f;

            // Two-pass population standard deviation for numerical stability.
            let variance: f64 = finite_valid_values
                .iter()
                .map(|&x| (x - mean).powi(2))
                .sum::<f64>()
                / count_f;
            let std_dev = variance.sqrt();

            finite_valid_values.sort_by(|a, b| a.total_cmp(b));
            let len = finite_valid_values.len();
            let median = if len % 2 == 1 {
                finite_valid_values[len / 2]
            } else {
                (finite_valid_values[len / 2 - 1] + finite_valid_values[len / 2]) / 2.0
            };

            // The values are already sorted for the median, and `percentile`
            // sorts a copy of what it is given, so this is four short scans
            // rather than four sorts.
            let percentiles = [0.05, 0.25, 0.75, 0.95].map(|fraction| {
                crate::percentile::percentile(&finite_valid_values, fraction)
                    .expect("the series is non-empty here")
            });

            let histogram = if min < max {
                let span = max - min;
                let bin_width = span / HISTOGRAM_BIN_COUNT as f64;
                let mut counts = vec![0usize; HISTOGRAM_BIN_COUNT];

                for &x in &finite_valid_values {
                    let mut bin = ((x - min) / bin_width).floor() as usize;
                    if bin >= HISTOGRAM_BIN_COUNT {
                        bin = HISTOGRAM_BIN_COUNT - 1;
                    }
                    counts[bin] += 1;
                }

                let bars = (0..HISTOGRAM_BIN_COUNT)
                    .map(|k| {
                        let center = min + (k as f64 + 0.5) * bin_width;
                        (center, counts[k] as f64, bin_width)
                    })
                    .collect();

                Some(HistogramData { bars })
            } else {
                None
            };

            (
                Some(min),
                Some(max),
                Some(peak_to_peak),
                Some(mean),
                Some(std_dev),
                Some(median),
                Some(percentiles),
                histogram,
            )
        };

    let time_axis = match file.master_channel(loc.data_group_index, loc.channel_group_index) {
        Some(master) => match file.signal(master).and_then(|s| s.values_f64()) {
            Ok(times) if !times.is_empty() => {
                let first = times[0];
                let last = times[times.len() - 1];
                let duration = last - first;
                let sample_rate = if duration > 0.0 && times.len() > 1 {
                    Some((times.len() - 1) as f64 / duration)
                } else {
                    None
                };
                Some(TimeAxisStats {
                    first_timestamp: first,
                    last_timestamp: last,
                    duration,
                    time_unit: master.unit.clone(),
                    sample_rate,
                })
            }
            _ => None,
        },
        None => None,
    };

    let counted_over = channel.array_shape().map(|shape| {
        let elements: u64 = shape.iter().product();
        format!(
            "An array channel: the figures below cover all {elements} elements of each of its {} samples, not one value per sample.",
            channel.sample_count
        )
    });

    StatsResult::Ok(Box::new(ChannelStats {
        channel_name: channel.name.clone(),
        channel_unit: channel.unit.clone(),
        counted_over,
        sample_count,
        valid_count,
        excluded_count,
        nan_inf_count,
        min,
        max,
        peak_to_peak,
        mean,
        std_dev,
        median,
        percentiles,
        first_value,
        last_value,
        time_axis,
        histogram,
    }))
}

fn format_f64(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x.is_sign_positive() {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        }
    } else {
        let s = format!("{x:.6}");
        if s.contains('.') {
            let trimmed = s.trim_end_matches('0');
            trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
        } else {
            s
        }
    }
}

fn format_value_with_unit(val: f64, unit: &str) -> String {
    let num = format_f64(val);
    if unit.is_empty() {
        num
    } else {
        format!("{num} {unit}")
    }
}

fn format_opt_value(val: Option<f64>, unit: &str) -> String {
    match val {
        Some(v) => format_value_with_unit(v, unit),
        None => "\u{2014}".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_f64_handles_special_and_regular_floats() {
        assert_eq!(format_f64(f64::NAN), "NaN");
        assert_eq!(format_f64(f64::INFINITY), "+Inf");
        assert_eq!(format_f64(f64::NEG_INFINITY), "-Inf");
        assert_eq!(format_f64(10.0), "10");
        assert_eq!(format_f64(10.5), "10.5");
        assert_eq!(format_f64(10.123456), "10.123456");
    }

    #[test]
    fn format_value_with_unit_formats_correctly() {
        assert_eq!(format_value_with_unit(5.0, "V"), "5 V");
        assert_eq!(format_value_with_unit(5.0, ""), "5");
    }

    #[test]
    fn format_opt_value_formats_correctly() {
        assert_eq!(format_opt_value(Some(5.0), "rpm"), "5 rpm");
        assert_eq!(format_opt_value(None, "rpm"), "\u{2014}");
    }
}
