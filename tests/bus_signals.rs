//! Decoded bus signals as time series, checked against the corpus.
//!
//! The central check is `series_match_the_frame_by_frame_loop`: `decode_bus` is
//! an accumulation loop the caller would otherwise write by hand, so the two
//! must agree reading for reading. It is run over both bus corpora, one keyed by
//! whole identifier and one by J1939 parameter group, because the two take
//! different paths through the database lookup.
//!
//! The remaining tests pin the properties that make the result a *series* rather
//! than a bag of numbers: readings in logging order, timestamps parallel to
//! values, multiplexed signals carrying only the frames that selected them, and
//! separate buses staying separate.
//!
//! The corpus is not checked in, so these tests skip when it is absent.

#![cfg(feature = "dbc")]

use std::collections::BTreeMap;
use std::path::Path;

use falcon_mdf::{CanDatabase, IdMatching, Mf4File};

const OBD2_LOG: &str =
    "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4";
const TRUCK_LOG: &str =
    "test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4";
const INTERNAL_LOG: &str = "test_data/mf4-sample-data-v2.1/Internal (CANedge GPS IMU)/LOG/\
     0BFD7754/00000014/00000014-64BBA8AF.MF4";

/// The SAE J1979 PIDs the Audi log carries, as in `tests/dbc_decoding.rs`.
const OBD2_DBC: &str = r#"VERSION "1"
NS_ :
BS_:
BU_: Tester ECU
BO_ 2024 OBD2: 8 ECU
 SG_ PID M : 23|8@0+ (1,0) [0|255] "" Tester
 SG_ CoolantTemperature m5 : 31|8@0+ (1,-40) [-40|215] "degC" Tester
 SG_ EngineSpeed m12 : 31|16@0+ (0.25,0) [0|16383.75] "rpm" Tester
 SG_ VehicleSpeed m13 : 31|8@0+ (1,0) [0|255] "km/h" Tester
"#;

/// EEC1 (parameter group `0xF004`) and total engine hours (`0xFEE5`), keyed the
/// way a J1939 DBC keys them: priority and source address included, and the
/// extended-identifier bit set.
///
/// Written from the constants rather than as decimal literals on purpose. A DBC
/// spells identifiers in decimal, and a mistyped digit here does not fail — PGN
/// matching simply finds whichever other parameter group the number names, and
/// the truck broadcasts a hundred of them.
const EEC1_ID: u32 = 0x8CF0_04FE;
const HOURS_ID: u32 = 0x98FE_E5FE;

fn j1939_dbc() -> String {
    format!(
        r#"VERSION "1"
NS_ :
BS_:
BU_: Vector__XXX
BO_ {EEC1_ID} EEC1: 8 Vector__XXX
 SG_ EngineTorqueMode : 0|4@1+ (1,0) [0|15] "" Vector__XXX
 SG_ EngineSpeed : 24|16@1+ (0.125,0) [0|8031.875] "rpm" Vector__XXX
BO_ {HOURS_ID} ENGINE_HOURS: 8 Vector__XXX
 SG_ TotalEngineHours : 0|32@1+ (0.05,0) [0|210554060.75] "h" Vector__XXX
VAL_ {EEC1_ID} EngineTorqueMode 0 "LowIdleGovernor" 1 "AcceleratorPedal" ;
"#
    )
}

/// Two identifiers from each bus of the multi-bus internal log.
const INTERNAL_DBC: &str = r#"VERSION "1"
NS_ :
BS_:
BU_: Logger
BO_ 1952 OnBusOne: 8 Logger
 SG_ ByteZero : 0|8@1+ (1,0) [0|255] "" Logger
BO_ 101 OnBusNine: 8 Logger
 SG_ ByteZero : 0|8@1+ (1,0) [0|255] "" Logger
"#;

fn skip(path: &str) -> bool {
    if Path::new(path).exists() {
        return false;
    }
    eprintln!("SKIP: {path} is absent");
    true
}

/// Every reading, accumulated the long way: loop the frames, call `decode`, and
/// append. This is the oracle — the code a caller writes when `decode_bus` does
/// not exist.
type Series = BTreeMap<(u8, String, String), (Vec<f64>, Vec<f64>)>;

fn frame_by_frame(path: &str, database: &CanDatabase) -> Series {
    let file = Mf4File::open(path).unwrap();
    let mut out: Series = BTreeMap::new();

    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            let Some(message) = database.message_name(frame.id).map(str::to_string) else {
                continue;
            };
            for signal in database.decode(frame.id, frame.data) {
                let entry = out
                    .entry((frame.bus_channel, message.clone(), signal.name.to_string()))
                    .or_default();
                entry.0.push(frame.timestamp);
                entry.1.push(signal.value);
            }
        }
    }
    out
}

/// The same readings as `decode_bus` reports them, in the same shape.
fn as_series(path: &str, database: &CanDatabase) -> Series {
    let file = Mf4File::open(path).unwrap();
    let mut out: Series = BTreeMap::new();

    for signal in file.decode_bus(database).unwrap().iter() {
        let key = (
            signal.bus_channel,
            signal.message.to_string(),
            signal.name.to_string(),
        );
        let previous = out.insert(key, (signal.timestamps.clone(), signal.values.clone()));
        assert!(previous.is_none(), "decode_bus reported a series twice");
    }
    out
}

/// `decode_bus` must produce exactly what the hand-written loop produces.
#[test]
fn series_match_the_frame_by_frame_loop() {
    let cases: [(&str, &str, IdMatching); 3] = [
        (OBD2_LOG, OBD2_DBC, IdMatching::Exact),
        (TRUCK_LOG, &j1939_dbc(), IdMatching::J1939Pgn),
        (INTERNAL_LOG, INTERNAL_DBC, IdMatching::Exact),
    ];

    for (path, dbc, matching) in cases {
        if skip(path) {
            continue;
        }
        let database = CanDatabase::from_dbc(dbc.as_bytes())
            .expect("the database must parse")
            .with_matching(matching);

        let expected = frame_by_frame(path, &database);
        let got = as_series(path, &database);

        assert!(!expected.is_empty(), "{path}: the oracle decoded nothing");
        assert_eq!(
            expected.keys().collect::<Vec<_>>(),
            got.keys().collect::<Vec<_>>(),
            "{path}: different signals"
        );
        for (key, (timestamps, values)) in &expected {
            let (got_timestamps, got_values) = &got[key];
            assert_eq!(timestamps, got_timestamps, "{path}: {key:?} timestamps");
            assert_eq!(values, got_values, "{path}: {key:?} values");
        }

        let readings: usize = expected.values().map(|(_, v)| v.len()).sum();
        eprintln!(
            "{}: {} series, {readings} readings agree",
            path.rsplit('/').next().unwrap(),
            expected.len()
        );
    }
}

/// The truck's series, pinned by length and by a physical range.
///
/// This exists because a mistyped identifier in the database above would not
/// fail any of the other tests: PGN matching would find a different parameter
/// group, decode it consistently through both paths, and agree with itself. Only
/// asking what the numbers mean catches that — engine hours read off the wrong
/// group came back as 78 million.
#[test]
fn the_truck_series_are_the_ones_the_database_names() {
    if skip(TRUCK_LOG) {
        return;
    }
    let database = CanDatabase::from_dbc(j1939_dbc().as_bytes())
        .unwrap()
        .with_matching(IdMatching::J1939Pgn);
    let file = Mf4File::open(TRUCK_LOG).unwrap();
    let signals = file.decode_bus(&database).unwrap();

    let lengths: BTreeMap<String, usize> = signals
        .iter()
        .map(|signal| (format!("{}.{}", signal.message, signal.name), signal.len()))
        .collect();
    assert_eq!(
        lengths,
        BTreeMap::from([
            // EEC1 from both ECUs that send it: 19 584 + 1 958.
            ("EEC1.EngineTorqueMode".to_string(), 21_542),
            ("EEC1.EngineSpeed".to_string(), 21_542),
            ("ENGINE_HOURS.TotalEngineHours".to_string(), 101),
        ])
    );

    let hours = signals.find("TotalEngineHours");
    let first = hours[0].values[0];
    assert!(
        (100.0..1_000_000.0).contains(&first),
        "a working truck's hour meter reads {first} h"
    );
}

/// A series is in logging order, and its two vectors are the same length. A
/// caller plotting values against timestamps depends on both.
#[test]
fn timestamps_are_parallel_and_ordered() {
    if skip(TRUCK_LOG) {
        return;
    }
    let database = CanDatabase::from_dbc(j1939_dbc().as_bytes())
        .unwrap()
        .with_matching(IdMatching::J1939Pgn);
    let file = Mf4File::open(TRUCK_LOG).unwrap();
    let signals = file.decode_bus(&database).unwrap();

    assert!(!signals.is_empty());
    for signal in signals.iter() {
        assert_eq!(
            signal.timestamps.len(),
            signal.values.len(),
            "{}.{}: timestamps and values are different lengths",
            signal.message,
            signal.name
        );
        assert_eq!(signal.len(), signal.values.len());
        assert!(!signal.is_empty(), "an empty series was reported");
        for pair in signal.timestamps.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "{}.{}: time went backwards, {} then {}",
                signal.message,
                signal.name,
                pair[0],
                pair[1]
            );
        }
    }
}

/// A multiplexed signal's series holds only the frames that selected it, so its
/// length is the number of responses carrying that PID — not the number of
/// frames on the identifier.
#[test]
fn a_multiplexed_signal_gets_only_its_own_readings() {
    if skip(OBD2_LOG) {
        return;
    }
    let database = CanDatabase::from_dbc(OBD2_DBC.as_bytes()).unwrap();
    let file = Mf4File::open(OBD2_LOG).unwrap();

    // Count the response frames per PID directly from the payloads.
    let mut per_pid: BTreeMap<u8, usize> = BTreeMap::new();
    let mut responses = 0usize;
    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            if frame.id == 0x7E8 {
                responses += 1;
                *per_pid.entry(frame.data[2]).or_default() += 1;
            }
        }
    }

    let signals = file.decode_bus(&database).unwrap();
    let length = |name: &str| -> usize {
        let found = signals.find(name);
        assert_eq!(found.len(), 1, "{name} should be one series");
        found[0].len()
    };

    // The multiplexor is decoded for every response; each branch only for its own.
    assert_eq!(length("PID"), responses);
    assert_eq!(length("EngineSpeed"), per_pid[&0x0C]);
    assert_eq!(length("CoolantTemperature"), per_pid[&0x05]);
    assert_eq!(length("VehicleSpeed"), per_pid[&0x0D]);
    assert!(
        length("EngineSpeed") < responses,
        "the branches cannot each hold every response"
    );
}

/// A multi-bus log keeps its buses apart, and each series says which bus it came
/// from rather than reporting a default.
#[test]
fn a_multi_bus_log_reports_the_bus_each_series_came_from() {
    if skip(INTERNAL_LOG) {
        return;
    }
    let database = CanDatabase::from_dbc(INTERNAL_DBC.as_bytes()).unwrap();
    let file = Mf4File::open(INTERNAL_LOG).unwrap();
    let signals = file.decode_bus(&database).unwrap();

    let buses: BTreeMap<&str, u8> = signals
        .iter()
        .map(|signal| (signal.message, signal.bus_channel))
        .collect();

    // The logger names its buses 1 and 9; these two messages are on one each.
    assert_eq!(buses.get("OnBusOne"), Some(&1));
    assert_eq!(buses.get("OnBusNine"), Some(&9));

    // Both messages define a signal called ByteZero, and they are not merged.
    assert_eq!(signals.find("ByteZero").len(), 2);
}

/// Value-table labels survive into the series and stay parallel to the values.
#[test]
fn labels_reach_the_series() {
    if skip(TRUCK_LOG) {
        return;
    }
    let database = CanDatabase::from_dbc(j1939_dbc().as_bytes())
        .unwrap()
        .with_matching(IdMatching::J1939Pgn);
    let file = Mf4File::open(TRUCK_LOG).unwrap();
    let signals = file.decode_bus(&database).unwrap();

    let modes = signals.find("EngineTorqueMode");
    assert!(!modes.is_empty());
    let mut labelled = 0usize;
    for mode in &modes {
        for index in 0..mode.len() {
            match mode.text_at(index) {
                Some(text) => {
                    // The table names 0 and 1 only, so a label implies the value.
                    let expected = if text == "LowIdleGovernor" { 0.0 } else { 1.0 };
                    assert_eq!(
                        mode.values[index], expected,
                        "{text} labelled the wrong value"
                    );
                    labelled += 1;
                }
                None => assert!(mode.values[index] > 1.0, "an unlabelled value in the table"),
            }
        }
    }
    assert!(labelled > 0, "no torque mode was labelled");

    // A signal with no value table reports no text for any reading.
    let speed = signals.find("EngineSpeed");
    assert_eq!(speed.len(), 1);
    assert!((0..speed[0].len()).all(|index| speed[0].text_at(index).is_none()));
}

/// A database that covers none of the traffic decodes to nothing, rather than to
/// empty series or to an error.
#[test]
fn a_database_covering_nothing_decodes_nothing() {
    if skip(TRUCK_LOG) {
        return;
    }
    let database = CanDatabase::new(Vec::new());
    let file = Mf4File::open(TRUCK_LOG).unwrap();
    let signals = file.decode_bus(&database).unwrap();

    assert!(signals.is_empty());
    assert_eq!(signals.len(), 0);
    assert!(signals.iter().next().is_none());
}
