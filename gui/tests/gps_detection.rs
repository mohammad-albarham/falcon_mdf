//! Tests for GPS position channel detection and pairing logic.

use falcon_mdf::{Mf4File, Mf4Writer};
use falcon_mdf_gui::panels::gps::{
    detect_gps_channels, is_latitude_channel_name, is_longitude_channel_name, GpsChannels,
};
use falcon_mdf_gui::model::ChannelLoc;

fn write_file(tag: &str, channels: &[(&str, Vec<f64>)]) -> Mf4File {
    let times: Vec<f64> = (0..channels[0].1.len()).map(|i| i as f64).collect();
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    for (name, values) in channels {
        group.add_channel(name, "u", values).unwrap();
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "falcon_gui_gps_{tag}_{}_{serial}.mf4",
        std::process::id()
    ));
    writer.write_to_file(&path).unwrap();
    let file = Mf4File::open(&path).expect("the written file should open");
    let _ = std::fs::remove_file(&path);
    file
}

fn write_multi_group_file(
    tag: &str,
    groups: &[&[(&str, Vec<f64>)]],
) -> Mf4File {
    let mut writer = Mf4Writer::new();
    for group_channels in groups {
        let times: Vec<f64> = (0..group_channels[0].1.len()).map(|i| i as f64).collect();
        let group = writer.add_group(&times).unwrap();
        for (name, values) in *group_channels {
            group.add_channel(name, "u", values).unwrap();
        }
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "falcon_gui_gps_mg_{tag}_{}_{serial}.mf4",
        std::process::id()
    ));
    writer.write_to_file(&path).unwrap();
    let file = Mf4File::open(&path).expect("the written file should open");
    let _ = std::fs::remove_file(&path);
    file
}

#[test]
fn detects_standard_latitude_and_longitude_channels() {
    let file = write_file(
        "standard",
        &[
            ("Latitude", vec![48.137154, 48.137200]),
            ("Longitude", vec![11.576124, 11.576200]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(
        detected,
        Some(GpsChannels {
            latitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 1, // 0 is time master
            },
            longitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 2,
            },
        })
    );
}

#[test]
fn detects_gps_prefixed_channels() {
    let file = write_file(
        "gps_prefix",
        &[
            ("GPS_Lat", vec![52.520008, 52.520100]),
            ("GPS_Long", vec![13.404954, 13.405000]),
            ("VehicleSpeed", vec![50.0, 52.0]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(
        detected,
        Some(GpsChannels {
            latitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 1,
            },
            longitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 2,
            },
        })
    );
}

#[test]
fn detects_pos_and_gnss_names() {
    let file = write_file(
        "pos_gnss",
        &[
            ("pos_lat", vec![37.7749, 37.7750]),
            ("pos_lon", vec![-122.4194, -122.4193]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(
        detected,
        Some(GpsChannels {
            latitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 1,
            },
            longitude: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 2,
            },
        })
    );
}

#[test]
fn file_with_no_position_channels_returns_none() {
    let file = write_file(
        "no_gps",
        &[
            ("EngineSpeed", vec![1000.0, 2000.0]),
            ("WheelSpeed", vec![20.0, 40.0]),
            ("Throttle", vec![10.0, 25.0]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(detected, None);
}

#[test]
fn vehicle_dynamics_channels_do_not_falsely_match() {
    let file = write_file(
        "dynamics",
        &[
            ("lat_accel", vec![0.1, 0.2]),
            ("long_accel", vec![0.5, 0.6]),
            ("lateral_velocity", vec![0.0, 0.1]),
            ("longitudinal_acceleration", vec![1.0, 1.2]),
            ("yaw_rate", vec![0.0, 0.05]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(detected, None);
}

#[test]
fn file_with_only_latitude_returns_none() {
    let file = write_file(
        "lat_only",
        &[
            ("Latitude", vec![48.0, 48.1]),
            ("EngineSpeed", vec![1500.0, 1600.0]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(detected, None);
}

#[test]
fn file_with_only_longitude_returns_none() {
    let file = write_file(
        "lon_only",
        &[
            ("Longitude", vec![11.0, 11.1]),
            ("EngineSpeed", vec![1500.0, 1600.0]),
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(detected, None);
}

#[test]
fn prefers_lat_lon_pair_in_same_channel_group() {
    // Group 0 has other channels + an orphaned Latitude
    // Group 1 has both GPS_Latitude and GPS_Longitude
    let file = write_multi_group_file(
        "same_group",
        &[
            &[
                ("EngineSpeed", vec![1000.0, 1100.0]),
                ("Other_Lat", vec![10.0, 11.0]),
            ],
            &[
                ("GPS_Latitude", vec![48.1, 48.2]),
                ("GPS_Longitude", vec![11.5, 11.6]),
            ],
        ],
    );
    let detected = detect_gps_channels(&file);
    assert_eq!(
        detected,
        Some(GpsChannels {
            latitude: ChannelLoc {
                data_group_index: 1,
                channel_group_index: 0,
                channel_index: 1,
            },
            longitude: ChannelLoc {
                data_group_index: 1,
                channel_group_index: 0,
                channel_index: 2,
            },
        })
    );
}

#[test]
fn name_matching_rules() {
    assert!(is_latitude_channel_name("lat"));
    assert!(is_latitude_channel_name("Latitude"));
    assert!(is_latitude_channel_name("GPS_Latitude"));
    assert!(is_latitude_channel_name("gps_lat"));
    assert!(is_latitude_channel_name("GPS.LAT"));
    assert!(is_latitude_channel_name("pos_lat"));
    assert!(is_latitude_channel_name("GNSS_Latitude_deg"));

    assert!(is_longitude_channel_name("lon"));
    assert!(is_longitude_channel_name("long"));
    assert!(is_longitude_channel_name("Longitude"));
    assert!(is_longitude_channel_name("GPS_Longitude"));
    assert!(is_longitude_channel_name("gps_lon"));
    assert!(is_longitude_channel_name("gps_long"));
    assert!(is_longitude_channel_name("GPS.LONG"));
    assert!(is_longitude_channel_name("pos_lon"));
    assert!(is_longitude_channel_name("GNSS_Longitude_deg"));

    // Dynamics are not GPS positions
    assert!(!is_latitude_channel_name("lat_accel"));
    assert!(!is_latitude_channel_name("lateral_acceleration"));
    assert!(!is_latitude_channel_name("lat_vel"));
    assert!(!is_longitude_channel_name("long_accel"));
    assert!(!is_longitude_channel_name("longitudinal_acceleration"));
    assert!(!is_longitude_channel_name("long_vel"));
}
