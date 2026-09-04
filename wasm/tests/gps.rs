//! Native tests for the GPS track binding (plan 3.4): pair detection by
//! name, the track payload, alignment policy, and the stride decimation
//! ceiling. The fixture is a writer-made "drive" around a small square —
//! the binding is channel-driven, so a synthetic path exercises exactly what
//! a real GNSS log would.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf::write::Mf4Writer;
use falcon_mdf_wasm::WasmMf4File;

/// A drive around a unit square at 10 samples per side: lat 55→56, lon
/// 10→11, with a speed ramp. Deterministic, so every coordinate is pinned.
fn gps_file(lat_values: &[f64], lon_values: &[f64], speed_values: &[f64]) -> WasmMf4File {
    let n = lat_values.len();
    let t: Vec<f64> = (0..n).map(|i| i as f64 * 0.5).collect();
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&t).expect("add group");
    group
        .add_channel("GPSLatitude", "deg", lat_values)
        .expect("lat");
    group
        .add_channel("GPSLongitude", "deg", lon_values)
        .expect("lon");
    // An empty speed list means "no speed channel at all", which the writer
    // cannot express as a zero-width channel.
    if !speed_values.is_empty() {
        group
            .add_channel("GPSSpeed", "km/h", speed_values)
            .expect("speed");
    }
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write MF4");
    // Keep a copy where the browser pass can deep-link the drive.
    let _ = std::fs::write("../test_data/generated/gen_gps.mdf", &bytes);
    let _ = std::fs::write("test_data/generated/gen_gps.mdf", &bytes);
    WasmMf4File::new(bytes).expect("open")
}

fn square_drive(n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let lats: Vec<f64> = (0..n).map(|i| 55.0 + (i % 20) as f64 / 20.0).collect();
    let lons: Vec<f64> = (0..n).map(|i| 10.0 + (i % 10) as f64 / 10.0).collect();
    let speed: Vec<f64> = (0..n).map(|i| 30.0 + i as f64).collect();
    (lats, lons, speed)
}

fn field_of(json: &str, key: &str) -> JsonVal {
    let JsonVal::Obj(fields) = parse_json(json).expect("valid json") else {
        panic!("payload is an object")
    };
    fields
        .iter()
        .find_map(|(k, v)| (k == key).then(|| v.clone()))
        .unwrap_or_else(|| panic!("no {key} in {json:?}"))
}

#[test]
fn detection_finds_the_coordinate_pair() {
    let file = gps_file(&[55.0], &[10.0], &[30.0]);
    let json = file.detect_gps_channels().expect("detection");
    let lat = field_of(&json, "latitude");
    let lon = field_of(&json, "longitude");
    assert_eq!(lat, JsonVal::Str("GPSLatitude".into()));
    assert_eq!(lon, JsonVal::Str("GPSLongitude".into()));
}

#[test]
fn detection_ignores_dynamics_and_non_position_names() {
    // A file with none of the position words detects nothing; a lateral
    // acceleration channel is not a latitude.
    let t = [0.0, 1.0, 2.0];
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&t).expect("add group");
    group
        .add_channel("LateralAccel", "", &[0.0; 3])
        .expect("ch");
    group
        .add_channel("VehicleSpeed", "", &[0.0; 3])
        .expect("ch");
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write");
    let file = WasmMf4File::new(bytes).expect("open");
    let json = file.detect_gps_channels().expect("detection");
    assert_eq!(field_of(&json, "latitude"), JsonVal::Null);
    assert_eq!(field_of(&json, "longitude"), JsonVal::Null);
}

#[test]
fn the_track_carries_the_expected_points() {
    let (lat, lon, speed) = square_drive(40);
    let mut file = gps_file(&lat, &lon, &speed);
    let json = file
        .gps_track("GPSLatitude", "GPSLongitude", Some("GPSSpeed".into()))
        .expect("track");
    let n = match field_of(&json, "n") {
        JsonVal::Number(n) => n as usize,
        other => panic!("n is a number, got {other:?}"),
    };
    assert_eq!(n, 40);
    // Sample 7: lat 55 + 7/20, lon 10 + 7/10, speed 30 + 7 — all pinned.
    let JsonVal::Array(lats) = field_of(&json, "lat") else {
        panic!("lat is an array")
    };
    let JsonVal::Array(lons) = field_of(&json, "lon") else {
        panic!("lon is an array")
    };
    let JsonVal::Array(speeds) = field_of(&json, "speed") else {
        panic!("speed is an array")
    };
    assert_eq!(lats.len(), 40);
    assert_eq!(lons.len(), 40);
    assert_eq!(speeds.len(), 40);
    assert_eq!(lats[7], JsonVal::Number(55.35));
    assert_eq!(lons[7], JsonVal::Number(10.7));
    assert_eq!(speeds[7], JsonVal::Number(37.0));
}

#[test]
fn track_decimates_long_drives_by_stride() {
    // 60k points against a 20k ceiling: the reply must carry at most 20k
    // points, evenly strided (first and last survive).
    let n = 60_000;
    let lat: Vec<f64> = (0..n).map(|i| 55.0 + i as f64 * 1e-5).collect();
    let lon: Vec<f64> = (0..n).map(|i| 10.0 + i as f64 * 1e-5).collect();
    let speed: Vec<f64> = (0..n).map(|i| 30.0).collect();
    let mut file = gps_file(&lat, &lon, &speed);
    let json = file
        .gps_track("GPSLatitude", "GPSLongitude", Some("GPSSpeed".into()))
        .expect("track");
    let n_out = match field_of(&json, "n") {
        JsonVal::Number(n) => n as usize,
        other => panic!("n is a number, got {other:?}"),
    };
    assert!(n_out <= 20_000, "ceiling respected: {n_out}");
    assert!(n_out >= 19_000, "stride keeps essentially every 3rd point");
}

#[test]
fn shorter_lon_is_reported_unaligned_and_gapped() {
    // A longitude that drops out early: the track keeps the latitude
    // timeline, pads the tail with nulls, and says it is unaligned.
    // Real GNSS logs can hold the coordinates in different groups with
    // different sample counts; the writer expresses that as two groups.
    let lat: Vec<f64> = (0..10).map(|i| 55.0 + i as f64).collect();
    let lon: Vec<f64> = (0..6).map(|i| 10.0 + i as f64).collect();
    let speed = [0.0; 6];
    let mut writer = Mf4Writer::new();
    let t10: Vec<f64> = (0..10).map(|i| i as f64 * 0.5).collect();
    let group_a = writer.add_group(&t10).expect("group A");
    group_a
        .add_channel("GPSLatitude", "deg", &lat)
        .expect("lat");
    let t6: Vec<f64> = (0..6).map(|i| i as f64 * 0.5).collect();
    let group_b = writer.add_group(&t6).expect("group B");
    group_b
        .add_channel("GPSLongitude", "deg", &lon)
        .expect("lon");
    group_b.add_channel("GPSSpeed", "km/h", &speed).expect("sp");
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write");
    let mut file = WasmMf4File::new(bytes).expect("open");
    let json = file
        .gps_track("GPSLatitude", "GPSLongitude", Some("GPSSpeed".into()))
        .expect("track");
    assert_eq!(field_of(&json, "aligned"), JsonVal::Bool(false));
    let JsonVal::Array(lons) = field_of(&json, "lon") else {
        panic!("lon is an array")
    };
    let nulls = lons.iter().filter(|v| **v == JsonVal::Null).count();
    assert!(nulls > 0, "the dropped tail is null gaps, not fake points");
}

#[test]
fn no_speed_channel_yields_a_null_speed_array() {
    let (lat, lon, _speed) = square_drive(10);
    let mut file = gps_file(&lat, &lon, &[]);
    let json = file
        .gps_track("GPSLatitude", "GPSLongitude", None)
        .expect("track");
    assert_eq!(field_of(&json, "speed"), JsonVal::Null);
}

#[test]
fn a_missing_channel_is_an_error() {
    let (lat, lon, speed) = square_drive(5);
    let mut file = gps_file(&lat, &lon, &speed);
    assert!(file
        .gps_track("GPSLatitude", "NoLon", Some("GPSSpeed".into()))
        .is_err());
}
