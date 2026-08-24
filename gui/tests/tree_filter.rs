//! Tests for structure tree filtering predicates in the falcon MF4 viewer.

use falcon_mdf_gui::panels::tree::{channel_matches, group_matches};

#[test]
fn empty_query_matches_everything() {
    // Channels
    assert!(channel_matches("EngineSpeed", "RPM", ""));
    assert!(channel_matches("", "", ""));
    assert!(channel_matches("VehicleSpeed", "km/h", "   "));

    // Groups
    let channels = [
        ("EngineSpeed".to_string(), "RPM".to_string()),
        ("VehicleSpeed".to_string(), "km/h".to_string()),
    ];
    assert!(group_matches("Powertrain", &channels, ""));
    assert!(group_matches("Powertrain", &channels, "   "));
    assert!(group_matches("", &[], ""));
}

#[test]
fn matching_is_case_insensitive() {
    // Channel name & unit
    assert!(channel_matches("EngineSpeed", "RPM", "enginespeed"));
    assert!(channel_matches("EngineSpeed", "RPM", "ENGINESPEED"));
    assert!(channel_matches("EngineSpeed", "rpm", "RPM"));
    assert!(channel_matches("EngineSpeed", "RPM", "rpm"));

    // Group name
    let channels = [("Channel_1".to_string(), "V".to_string())];
    assert!(group_matches("CAN_BUS_1", &channels, "can_bus_1"));
    assert!(group_matches("can_bus_1", &channels, "CAN_BUS_1"));
    assert!(group_matches("Can_Bus_1", &channels, "CAN"));
}

#[test]
fn channel_matches_on_unit_as_well_as_name() {
    assert!(
        channel_matches("VehicleSpeed", "km/h", "km/h"),
        "channel should match on exact unit"
    );
    assert!(
        channel_matches("BatteryVoltage", "Volt", "volt"),
        "channel should match on unit substring case-insensitively"
    );
    assert!(
        channel_matches("Temperature", "degC", "degc"),
        "channel should match on temperature unit"
    );
    assert!(
        !channel_matches("VehicleSpeed", "km/h", "RPM"),
        "channel should not match non-existent query in name or unit"
    );
}

#[test]
fn group_matches_when_only_one_channel_matches() {
    let channels = [
        ("WheelSpeed_FL".to_string(), "km/h".to_string()),
        ("WheelSpeed_FR".to_string(), "km/h".to_string()),
        ("SteeringAngle".to_string(), "deg".to_string()),
        ("BrakePressure".to_string(), "bar".to_string()),
    ];

    // Query matches only SteeringAngle
    assert!(
        group_matches("ChassisData", &channels, "steering"),
        "group should match when only one of its channels matches"
    );
    // Query matches only BrakePressure by unit
    assert!(
        group_matches("ChassisData", &channels, "bar"),
        "group should match when only one channel's unit matches"
    );
}

#[test]
fn group_matches_on_own_name_even_when_no_channel_matches() {
    let channels = [
        ("Alpha".to_string(), "m".to_string()),
        ("Beta".to_string(), "s".to_string()),
    ];

    assert!(
        group_matches("PowertrainCAN", &channels, "powertrain"),
        "group must match when its acquisition name matches despite no channel match"
    );
    assert!(
        group_matches("Telemetry_HighRate", &[], "telemetry"),
        "group with no channels must still match if its name matches"
    );
}

#[test]
fn group_with_no_matching_channel_and_non_matching_name_does_not_match() {
    let channels = [
        ("EngineSpeed".to_string(), "RPM".to_string()),
        ("CoolantTemp".to_string(), "degC".to_string()),
    ];

    assert!(
        !group_matches("EngineGroup", &channels, "Transmission"),
        "group with neither name nor channel matching must return false"
    );
    assert!(
        !group_matches("SensorBlock", &[], "Pressure"),
        "empty group with non-matching name must return false"
    );
}

#[test]
fn whitespace_around_query_is_ignored() {
    assert!(
        channel_matches("EngineSpeed", "RPM", "  speed  "),
        "leading/trailing whitespace around channel query should be ignored"
    );
    assert!(
        channel_matches("EngineSpeed", "RPM", " \t rpm \n "),
        "tabs and newlines around channel query should be trimmed"
    );

    let channels = [("Pressure".to_string(), "bar".to_string())];
    assert!(
        group_matches("Hydraulics", &channels, "  hydraulics  "),
        "whitespace around group name query should be trimmed"
    );
    assert!(
        group_matches("Hydraulics", &channels, "  bar  "),
        "whitespace around channel unit query in group should be trimmed"
    );
}

#[test]
fn query_matching_nothing_matches_nothing() {
    let channels = [
        ("ChannelA".to_string(), "V".to_string()),
        ("ChannelB".to_string(), "A".to_string()),
    ];

    assert!(
        !channel_matches("ChannelA", "V", "xyz_nonexistent"),
        "non-matching query against channel should return false"
    );
    assert!(
        !group_matches("Group1", &channels, "xyz_nonexistent"),
        "non-matching query against group should return false"
    );
}
