//! The GPS map panel: plots latitude and longitude position channels as a
//! 2D track polyline.
//!
//! Position channels are detected automatically from channel names in the
//! open measurement, with combo boxes allowing manual override if detection
//! fails or if another coordinate pair is desired. Measurement cursors are
//! shared with the time plot so cursor positions are marked on the track.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use egui_plot::{Line, Plot, Points};
use falcon_mdf::Mf4File;

use crate::model::{ChannelLoc, LoadedFile, Row};
use crate::signal_loader::{spawn_signal_load, ChannelSignal, SignalLoadResult};
use crate::xy::{pair_xy, XyRefusal, XySeries};

/// One position channel's decode state.
enum Slot {
    Loading(Receiver<SignalLoadResult>),
    Loaded(ChannelSignal),
    Failed(String),
}

const CURSOR_A_COLOR: egui::Color32 = egui::Color32::from_rgb(0x33, 0x99, 0xff);
const CURSOR_B_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x99, 0x00);
const TRACK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x77, 0xb4);
const REFUSAL_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 80, 80);

/// The detected or chosen latitude and longitude channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsChannels {
    pub latitude: ChannelLoc,
    pub longitude: ChannelLoc,
}

pub struct GpsPanel {
    /// Decoded coordinate channels.
    slots: HashMap<ChannelLoc, Slot>,
    /// Selected latitude channel.
    lat_channel: Option<ChannelLoc>,
    /// Selected longitude channel.
    lon_channel: Option<ChannelLoc>,
    /// Whether automatic detection has run for the current file.
    detected: bool,
    /// Whether individual sample points are drawn along the track polyline.
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
            lat_channel: None,
            lon_channel: None,
            detected: false,
            show_points: false,
        }
    }

    pub fn reset(&mut self) {
        self.slots.clear();
        self.lat_channel = None;
        self.lon_channel = None;
        self.detected = false;
    }

    pub fn channels(&self) -> Option<GpsChannels> {
        match (self.lat_channel, self.lon_channel) {
            (Some(latitude), Some(longitude)) => Some(GpsChannels {
                latitude,
                longitude,
            }),
            _ => None,
        }
    }

    pub fn set_channels(&mut self, channels: Option<GpsChannels>) {
        self.lat_channel = channels.map(|c| c.latitude);
        self.lon_channel = channels.map(|c| c.longitude);
        self.detected = true;
        self.slots.clear();
    }

    fn sync_slots(&mut self, ui: &egui::Ui, file: &Arc<Mf4File>) {
        let needed = [self.lat_channel, self.lon_channel];
        for loc_opt in needed {
            let Some(loc) = loc_opt else { continue };
            if self.slots.contains_key(&loc) {
                continue;
            }
            self.slots.insert(
                loc,
                Slot::Loading(spawn_signal_load(
                    Arc::clone(file),
                    loc,
                    ui.ctx().clone(),
                )),
            );
        }
        self.slots.retain(|loc, _| {
            Some(*loc) == self.lat_channel || Some(*loc) == self.lon_channel
        });
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

    fn show_channel_pickers(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
    ) {
        let mut all_channels: Vec<(ChannelLoc, &str)> = Vec::new();
        for row in &loaded.all_rows {
            if let Row::Channel {
                loc,
                name,
                unreadable,
                ..
            } = row
            {
                if unreadable.is_none() {
                    all_channels.push((*loc, name.as_str()));
                }
            }
        }

        let lat_text = self
            .lat_channel
            .and_then(|loc| channel_name_at(&loaded.file, loc))
            .unwrap_or_else(|| "(pick latitude)".to_string());

        let lon_text = self
            .lon_channel
            .and_then(|loc| channel_name_at(&loaded.file, loc))
            .unwrap_or_else(|| "(pick longitude)".to_string());

        ui.horizontal_wrapped(|ui| {
            ui.label("Latitude:");
            egui::ComboBox::from_id_salt("gps_lat_combo")
                .selected_text(lat_text)
                .show_ui(ui, |ui| {
                    for (loc, name) in &all_channels {
                        let is_selected = self.lat_channel == Some(*loc);
                        if ui.selectable_label(is_selected, *name).clicked() {
                            self.lat_channel = Some(*loc);
                        }
                    }
                });

            ui.add_space(8.0);
            ui.label("Longitude:");
            egui::ComboBox::from_id_salt("gps_lon_combo")
                .selected_text(lon_text)
                .show_ui(ui, |ui| {
                    for (loc, name) in &all_channels {
                        let is_selected = self.lon_channel == Some(*loc);
                        if ui.selectable_label(is_selected, *name).clicked() {
                            self.lon_channel = Some(*loc);
                        }
                    }
                });

            ui.separator();
            ui.checkbox(&mut self.show_points, "Samples")
                .on_hover_text("Draw the position samples on top of the track");
        });
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        if !self.detected {
            self.detected = true;
            if let Some(pair) = detect_gps_channels(&loaded.file) {
                self.lat_channel = Some(pair.latitude);
                self.lon_channel = Some(pair.longitude);
            }
        }

        self.show_channel_pickers(ui, loaded);
        self.sync_slots(ui, &loaded.file);
        self.poll();
        ui.separator();

        let (Some(lat_loc), Some(lon_loc)) = (self.lat_channel, self.lon_channel) else {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("No GPS position channels detected");
                ui.label(
                    "This measurement has no recognized latitude/longitude channels. \
                     Pick channels in the combo boxes above to plot a track.",
                );
            });
            return;
        };

        if lat_loc == lon_loc {
            ui.label(
                "Latitude and Longitude are set to the same channel. \
                 Pick a different channel for one of the coordinates.",
            );
            return;
        }

        let lat_slot = self.slots.get(&lat_loc);
        let lon_slot = self.slots.get(&lon_loc);

        let (lat_signal, lon_signal) = match (lat_slot, lon_slot) {
            (Some(Slot::Loaded(lat_sig)), Some(Slot::Loaded(lon_sig))) => (lat_sig, lon_sig),
            (Some(Slot::Failed(msg)), _) => {
                ui.colored_label(REFUSAL_COLOR, format!("Latitude: {msg}"));
                return;
            }
            (_, Some(Slot::Failed(msg))) => {
                ui.colored_label(REFUSAL_COLOR, format!("Longitude: {msg}"));
                return;
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Decoding GPS channels\u{2026}");
                });
                return;
            }
        };

        // Pair with Longitude on X and Latitude on Y
        let paired = pair_xy(lon_signal, 0.0, lat_signal, 0.0, false, false);
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
        series: &XySeries,
        lat_signal: &ChannelSignal,
        lon_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        let marker_a = cursor_a.and_then(|t| series.point_at(t));
        let marker_b = cursor_b.and_then(|t| series.point_at(t));

        let points = series.points.clone();
        let show_points = self.show_points || points.len() < 2;

        Plot::new("gps_plot")
            .data_aspect(1.0)
            .x_axis_label(axis_label(&lon_signal.name, &lon_signal.unit))
            .y_axis_label(axis_label(&lat_signal.name, &lat_signal.unit))
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Track", points.clone())
                        .color(TRACK_COLOR)
                        .width(1.5),
                );
                if show_points {
                    plot_ui.points(
                        Points::new("samples", points)
                            .color(TRACK_COLOR)
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
                "{} position sample(s) left out: invalid or NaN on one axis.",
                series.dropped
            ));
        }

        self.show_cursor_readout(ui, series, lat_signal, lon_signal, cursor_a, cursor_b);
    }

    fn show_cursor_readout(
        &self,
        ui: &mut egui::Ui,
        series: &XySeries,
        lat_signal: &ChannelSignal,
        lon_signal: &ChannelSignal,
        cursor_a: Option<f64>,
        cursor_b: Option<f64>,
    ) {
        if cursor_a.is_none() && cursor_b.is_none() {
            ui.weak(
                "Place the measurement cursors in the Plot tab to mark where the position was at \
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
                        Some(t) => match series.point_at(t) {
                            Some(m) => {
                                let drift = (m.time - t).abs();
                                if drift > 1e-6 {
                                    ui.label(format!("{:.6} s (cursor at {t:.6})", m.time))
                                        .on_hover_text(
                                            "The nearest sample is this far from the cursor: the \
                                             track has a gap there.",
                                        );
                                } else {
                                    ui.label(format!("{:.6} s", m.time));
                                }
                                ui.label(format!("{:.6}", m.point[0]));
                                ui.label(format!("{:.6}", m.point[1]));
                            }
                            None => {
                                let span = series.span();
                                let detail = match span {
                                    Some((lo, hi)) => format!(
                                        "outside the recorded span ({lo:.6}\u{2026}{hi:.6} s)"
                                    ),
                                    None => "outside the recorded span".to_string(),
                                };
                                ui.weak(detail);
                                ui.label("\u{2014}");
                                ui.label("\u{2014}");
                            }
                        },
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

/// Detects latitude and longitude channels in `file` by name.
///
/// Looks for a pair in the same channel group first (sharing a raster),
/// then falls back to any readable latitude and longitude channels in the file.
pub fn detect_gps_channels(file: &Mf4File) -> Option<GpsChannels> {
    // 1. Same channel group search
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            let mut lat = None;
            let mut lon = None;
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if ch.unreadable().is_some() {
                    continue;
                }
                let loc = ChannelLoc {
                    data_group_index: dg_idx,
                    channel_group_index: cg_idx,
                    channel_index: ch_idx,
                };
                if lat.is_none() && is_latitude_channel_name(&ch.name) {
                    lat = Some(loc);
                } else if lon.is_none() && is_longitude_channel_name(&ch.name) {
                    lon = Some(loc);
                }
            }
            if let (Some(latitude), Some(longitude)) = (lat, lon) {
                return Some(GpsChannels {
                    latitude,
                    longitude,
                });
            }
        }
    }

    // 2. Global search across the file
    let mut first_lat = None;
    let mut first_lon = None;
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if ch.unreadable().is_some() {
                    continue;
                }
                let loc = ChannelLoc {
                    data_group_index: dg_idx,
                    channel_group_index: cg_idx,
                    channel_index: ch_idx,
                };
                if first_lat.is_none() && is_latitude_channel_name(&ch.name) {
                    first_lat = Some(loc);
                }
                if first_lon.is_none() && is_longitude_channel_name(&ch.name) {
                    first_lon = Some(loc);
                }
            }
        }
    }

    match (first_lat, first_lon) {
        (Some(latitude), Some(longitude)) => Some(GpsChannels {
            latitude,
            longitude,
        }),
        _ => None,
    }
}

/// Whether `name` indicates a latitude channel.
pub fn is_latitude_channel_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    // Disqualify dynamics, errors, and non-coordinate signals.
    if is_disqualified_position_name(&lower) || lower.contains("lateral") {
        return false;
    }

    // Direct match
    if lower == "lat" || lower == "latitude" {
        return true;
    }

    // Tokenized check
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.contains(&"latitude") || tokens.contains(&"lat") {
        return true;
    }

    // Stripped common prefix check (e.g. gpslatitude, gpslat, poslatitude, poslat, gnsslat, navlat)
    for prefix in [
        "gps", "gnss", "pos", "position", "nav", "rt", "vbox", "ins", "can_gps", "vehicle_gps",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
            if rest == "lat"
                || rest == "latitude"
                || rest.starts_with("lat_")
                || rest.starts_with("latitude_")
                || rest == "latdeg"
                || rest == "latdegrees"
                || rest == "latitudedeg"
                || rest == "latitudedegrees"
            {
                return true;
            }
        }
    }

    false
}

/// Whether `name` indicates a longitude channel.
pub fn is_longitude_channel_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    // Disqualify dynamics, errors, and non-coordinate signals.
    if is_disqualified_position_name(&lower) || lower.contains("longitudinal") {
        return false;
    }

    // Direct match
    if lower == "lon" || lower == "long" || lower == "longitude" {
        return true;
    }

    // Tokenized check
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.contains(&"longitude") || tokens.contains(&"lon") || tokens.contains(&"long") {
        return true;
    }

    // Stripped common prefix check (e.g. gpslongitude, gpslon, gpslong, poslongitude, poslon, poslong)
    for prefix in [
        "gps", "gnss", "pos", "position", "nav", "rt", "vbox", "ins", "can_gps", "vehicle_gps",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
            if rest == "lon"
                || rest == "long"
                || rest == "longitude"
                || rest.starts_with("lon_")
                || rest.starts_with("long_")
                || rest.starts_with("longitude_")
                || rest == "londeg"
                || rest == "longdeg"
                || rest == "longitudedeg"
            {
                return true;
            }
        }
    }

    false
}

/// Keywords that disqualify a channel from being a geographic position.
fn is_disqualified_position_name(lower: &str) -> bool {
    const DISQUALIFIERS: &[&str] = &[
        "accel",
        "acceleration",
        "rate",
        "vel",
        "velocity",
        "speed",
        "force",
        "jerk",
        "error",
        "err",
        "std",
        "sigma",
        "variance",
        "accuracy",
        "quality",
        "status",
        "valid",
        "flag",
        "satellites",
        "heading",
        "course",
        "altitude",
        "elev",
        "elevation",
        "dist",
        "distance",
        "offset",
        "count",
        "num",
    ];

    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    for dq in DISQUALIFIERS {
        if tokens.contains(dq) {
            return true;
        }
    }

    // Substrings for compound names like "lataccel", "longvel", "yawrate", etc.
    if lower.contains("accel")
        || lower.contains("acceleration")
        || lower.contains("velocity")
        || lower.contains("rate")
    {
        return true;
    }

    false
}

fn show_refusal(ui: &mut egui::Ui, refusal: &XyRefusal) {
    ui.add_space(20.0);
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(REFUSAL_COLOR, "\u{26a0}");
        ui.colored_label(
            REFUSAL_COLOR,
            "These position channels cannot be plotted together.",
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

fn channel_name_at(file: &Mf4File, loc: ChannelLoc) -> Option<String> {
    file.data_groups()
        .get(loc.data_group_index)?
        .channel_groups
        .get(loc.channel_group_index)?
        .channels
        .get(loc.channel_index)
        .map(|c| c.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latitude_detection_names() {
        let positive = [
            "lat",
            "Lat",
            "LAT",
            "latitude",
            "Latitude",
            "LATITUDE",
            "GPS_Lat",
            "GPS_Latitude",
            "gps_latitude",
            "GPS.LAT",
            "GPS:Latitude",
            "GPS-Latitude",
            "POS_Lat",
            "Pos_Latitude",
            "GNSS_Lat",
            "GNSS_Latitude",
            "Nav_Lat",
            "Nav_Latitude",
            "GPSLat",
            "GPSLatitude",
            "PosLat",
            "PositionLatitude",
            "Vehicle_GPS_Latitude",
            "CAN_GPS_Lat",
            "Latitude_deg",
            "GPS_Lat_deg",
        ];
        for name in positive {
            assert!(
                is_latitude_channel_name(name),
                "expected '{name}' to be recognized as latitude"
            );
        }

        let negative = [
            "lat_accel",
            "LatAccel",
            "lateral_acceleration",
            "lat_rate",
            "lat_vel",
            "lateral_velocity",
            "gps_speed",
            "gps_altitude",
            "gps_heading",
            "gps_satellites",
            "gps_time",
            "EngineSpeed",
            "VehicleSpeed",
            "BatteryVoltage",
            "",
        ];
        for name in negative {
            assert!(
                !is_latitude_channel_name(name),
                "expected '{name}' NOT to be recognized as latitude"
            );
        }
    }

    #[test]
    fn test_longitude_detection_names() {
        let positive = [
            "lon",
            "Lon",
            "LON",
            "long",
            "Long",
            "LONG",
            "longitude",
            "Longitude",
            "LONGITUDE",
            "GPS_Lon",
            "GPS_Long",
            "GPS_Longitude",
            "gps_longitude",
            "GPS.LON",
            "GPS:Longitude",
            "GPS-Longitude",
            "POS_Lon",
            "Pos_Long",
            "Pos_Longitude",
            "GNSS_Lon",
            "GNSS_Long",
            "GNSS_Longitude",
            "Nav_Lon",
            "Nav_Longitude",
            "GPSLon",
            "GPSLong",
            "GPSLongitude",
            "PosLon",
            "PositionLongitude",
            "Vehicle_GPS_Longitude",
            "CAN_GPS_Long",
            "Longitude_deg",
            "GPS_Lon_deg",
        ];
        for name in positive {
            assert!(
                is_longitude_channel_name(name),
                "expected '{name}' to be recognized as longitude"
            );
        }

        let negative = [
            "long_accel",
            "LongAccel",
            "longitudinal_acceleration",
            "long_vel",
            "longitudinal_velocity",
            "gps_speed",
            "gps_altitude",
            "gps_heading",
            "gps_satellites",
            "gps_time",
            "EngineSpeed",
            "VehicleSpeed",
            "",
        ];
        for name in negative {
            assert!(
                !is_longitude_channel_name(name),
                "expected '{name}' NOT to be recognized as longitude"
            );
        }
    }
}
