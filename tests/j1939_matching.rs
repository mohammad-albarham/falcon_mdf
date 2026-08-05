//! J1939 parameter-group matching, checked against a real truck log.
//!
//! A J1939 database keys its messages by parameter group number, but the
//! identifier on the wire carries a priority and the sending ECU's source
//! address around it. Matching the whole identifier therefore decodes a real
//! heavy-duty log to nothing — which is what the first test here demonstrates,
//! on the corpus, before the rest show the matching mode fixing it.
//!
//! The database below is written from SAE J1939-71, not from this crate's
//! output: EEC1 is parameter group 61444 with engine speed in bytes 4 and 5 at
//! 0.125 rpm per bit, and total engine hours is group 65253 at 0.05 h per bit.
//! The identifiers are spelled the way a published J1939 DBC spells them, with
//! source address `0xFE` — the null address, which no ECU in this log uses.
//!
//! The corpus is not checked in, so these tests skip when it is absent.

#![cfg(feature = "dbc")]

use falcon_mdf::{CanDatabase, IdMatching, Mf4File};
use std::path::Path;

const TRUCK_LOG: &str =
    "test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4";

/// EEC1 as a J1939 DBC writes it, with the extended-identifier bit set.
/// `0x8CF004FE` is priority 3, parameter group `0xF004`, source address `0xFE`.
const EEC1_ID: u32 = 0x8CF0_04FE;
/// Total engine hours: priority 6, parameter group `0xFEE5`, source `0xFE`.
const HOURS_ID: u32 = 0x98FE_E5FE;

/// The identifiers the truck actually transmits these groups on, both source
/// addresses for EEC1 — no database keyed to one ECU can cover both.
const EEC1_FRAMES: [u32; 2] = [0x0CF0_0400, 0x0CF0_0421];
const HOURS_FRAME: u32 = 0x18FE_E500;

fn j1939_dbc() -> String {
    format!(
        "VERSION \"1\"
NS_ :
BS_:
BU_: Vector__XXX
BO_ {EEC1_ID} EEC1: 8 Vector__XXX
 SG_ EngineTorqueMode : 0|4@1+ (1,0) [0|15] \"\" Vector__XXX
 SG_ DriversDemandEnginePercentTorque : 8|8@1+ (1,-125) [-125|125] \"%\" Vector__XXX
 SG_ ActualEnginePercentTorque : 16|8@1+ (1,-125) [-125|125] \"%\" Vector__XXX
 SG_ EngineSpeed : 24|16@1+ (0.125,0) [0|8031.875] \"rpm\" Vector__XXX
BO_ {HOURS_ID} ENGINE_HOURS: 8 Vector__XXX
 SG_ TotalEngineHours : 0|32@1+ (0.05,0) [0|210554060.75] \"h\" Vector__XXX
VAL_ {EEC1_ID} EngineTorqueMode 0 \"LowIdleGovernor\" 1 \"AcceleratorPedal\" 2 \"CruiseControl\" 3 \"PTOGovernor\" 4 \"RoadSpeedGovernor\" 5 \"ASRControl\" ;
"
    )
}

fn database(matching: IdMatching) -> CanDatabase {
    CanDatabase::from_dbc(j1939_dbc().as_bytes())
        .expect("the J1939 database must parse")
        .with_matching(matching)
}

fn skip() -> bool {
    if Path::new(TRUCK_LOG).exists() {
        return false;
    }
    eprintln!("SKIP: {TRUCK_LOG} is absent");
    true
}

/// Every frame in the log, as `(timestamp, identifier, payload)`.
fn frames() -> Vec<(f64, u32, Vec<u8>)> {
    let file = Mf4File::open(TRUCK_LOG).unwrap();
    let mut out = Vec::new();
    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            out.push((frame.timestamp, frame.id, frame.data.to_vec()));
        }
    }
    out
}

/// The defect itself: a correctly written J1939 database, matched by whole
/// identifier, decodes an entire truck log to nothing.
///
/// This is the test that justifies T3 existing. It asserts a failure, so it has
/// to be specific about *why* the failure happens — the database does hold EEC1,
/// and the log does carry it; only the source address differs.
#[test]
fn exact_matching_decodes_a_real_j1939_log_to_nothing() {
    if skip() {
        return;
    }
    let db = database(IdMatching::Exact);

    // The database really does contain both groups, under the null address.
    assert_eq!(db.message_name(EEC1_ID), Some("EEC1"));
    assert_eq!(db.message_name(HOURS_ID), Some("ENGINE_HOURS"));

    let decoded: usize = frames()
        .iter()
        .map(|(_, id, payload)| db.decode(*id, payload).len())
        .sum();
    assert_eq!(
        decoded, 0,
        "exact matching decoded {decoded} signals; it should decode none, \
         because no ECU in this log transmits from source address 0xFE"
    );
}

/// The same database under PGN matching finds the group from both ECUs that
/// send it. Two source addresses is the case a "pin the address you saw"
/// workaround cannot handle.
#[test]
fn pgn_matching_finds_the_group_from_every_ecu() {
    if skip() {
        return;
    }
    let db = database(IdMatching::J1939Pgn);

    let mut per_id = std::collections::BTreeMap::new();
    for (_, id, payload) in frames() {
        if !db.decode(id, &payload).is_empty() {
            *per_id.entry(id).or_insert(0usize) += 1;
        }
    }

    for id in EEC1_FRAMES {
        assert!(
            per_id.contains_key(&id),
            "EEC1 from {id:#010X} decoded nothing; matched {per_id:#X?}"
        );
        assert_eq!(db.message_name(id), Some("EEC1"));
    }
    assert!(per_id.contains_key(&HOURS_FRAME));

    // The counts are the ones tests/bus_frames.rs already sees in this file, so
    // a change in frame assembly shows up here too rather than only there.
    assert_eq!(per_id[&EEC1_FRAMES[0]], 19_584);
    assert_eq!(per_id[&EEC1_FRAMES[1]], 1_958);
    assert_eq!(per_id[&HOURS_FRAME], 101);

    // And nothing beyond the two groups the database covers was matched.
    let matched: std::collections::BTreeSet<u32> = per_id.keys().copied().collect();
    let expected: std::collections::BTreeSet<u32> =
        EEC1_FRAMES.into_iter().chain([HOURS_FRAME]).collect();
    assert_eq!(matched, expected, "PGN matching over-matched");
}

/// Engine speed decoded against the J1939-71 definition, computed here from the
/// raw bytes rather than taken from the decoder — and then checked for physical
/// plausibility, which is what catches a self-consistent reader of wrong bits.
#[test]
fn engine_speed_matches_the_published_definition() {
    if skip() {
        return;
    }
    let db = database(IdMatching::J1939Pgn);
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut checked = 0usize;

    for (_, id, payload) in frames() {
        if !EEC1_FRAMES.contains(&id) {
            continue;
        }
        let decoded = db.decode(id, &payload);
        let speed = decoded
            .iter()
            .find(|s| s.name == "EngineSpeed")
            .expect("EEC1 must decode engine speed");

        // J1939-71 SPN 190: bytes 4-5, little-endian, 0.125 rpm per bit.
        let want = (f64::from(payload[3]) + f64::from(payload[4]) * 256.0) * 0.125;
        assert!(
            (speed.value - want).abs() < 1e-9,
            "decoded {} rpm from {:02X?}, the definition gives {want}",
            speed.value,
            payload
        );
        assert_eq!(speed.unit, "rpm");

        // 0xFFFF is J1939's "not available"; it is not a reading.
        if payload[3] != 0xFF || payload[4] != 0xFF {
            low = low.min(speed.value);
            high = high.max(speed.value);
        }
        checked += 1;
    }

    assert_eq!(checked, 21_542, "every EEC1 frame must decode");
    assert!(
        low >= 0.0 && high < 3500.0,
        "engine speed ranged {low}..{high} rpm, which is not a diesel truck"
    );
    assert!(high > 600.0, "the engine never turned: {high} rpm");
    eprintln!("engine speed ranged {low}..{high} rpm over {checked} frames");
}

/// Total engine hours is a 32-bit odometer. It can only go up, and for a working
/// truck it is a large number — two properties no fixture supplies and a decoder
/// reading the wrong bits or the wrong byte order will not satisfy.
#[test]
fn total_engine_hours_only_increases() {
    if skip() {
        return;
    }
    let db = database(IdMatching::J1939Pgn);
    let mut readings: Vec<f64> = Vec::new();

    for (_, id, payload) in frames() {
        if id != HOURS_FRAME {
            continue;
        }
        for signal in db.decode(id, &payload) {
            if signal.name == "TotalEngineHours" {
                assert_eq!(signal.unit, "h");
                readings.push(signal.value);
            }
        }
    }

    assert_eq!(readings.len(), 101);
    for pair in readings.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "the hour meter ran backwards: {} then {}",
            pair[0],
            pair[1]
        );
    }

    let (first, last) = (readings[0], readings[readings.len() - 1]);
    // A truck in service, not a factory-fresh ECU and not the 0.05 h/bit scale
    // applied to the wrong width: reading this field big-endian instead turns
    // roughly ten thousand hours into billions.
    assert!(
        (100.0..1_000_000.0).contains(&first),
        "total engine hours reads {first} h"
    );
    assert!(
        last - first < 1.0,
        "the meter advanced {} h during a log a few minutes long",
        last - first
    );
    eprintln!(
        "total engine hours: {first} h over {} readings",
        readings.len()
    );
}

/// The value table T4 adds, exercised on real traffic: every torque mode the
/// truck reports must be one the published table names, and the label must
/// follow the raw four-bit field rather than any scaled value.
#[test]
fn torque_mode_labels_follow_the_raw_field() {
    if skip() {
        return;
    }
    let db = database(IdMatching::J1939Pgn);
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    let mut unlabelled = 0usize;

    for (_, id, payload) in frames() {
        if !EEC1_FRAMES.contains(&id) {
            continue;
        }
        let decoded = db.decode(id, &payload);
        let mode = decoded
            .iter()
            .find(|s| s.name == "EngineTorqueMode")
            .expect("EEC1 must decode the torque mode");

        // The raw field is the low nibble of byte 0, and the label must be the
        // one the table gives that number — not the one for its scaled value,
        // which is the same here only because the factor is 1.
        let raw = payload[0] & 0x0F;
        assert_eq!(mode.value, f64::from(raw));

        match mode.text {
            Some(text) => *seen.entry(text.to_string()).or_default() += 1,
            None => {
                // Only values outside the table may be unlabelled, and the table
                // covers 0..=5.
                assert!(raw > 5, "raw mode {raw} is in the table but unlabelled");
                unlabelled += 1;
            }
        }
    }

    assert!(
        !seen.is_empty(),
        "no torque mode was labelled at all; {unlabelled} were left unlabelled"
    );
    eprintln!("torque modes seen: {seen:?}, unlabelled: {unlabelled}");
}
