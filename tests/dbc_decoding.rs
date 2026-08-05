//! DBC decoding checked against a real bus log and an outside specification.
//!
//! The database below is written from the published SAE J1979 / ISO 15031-5 PID
//! table — mode `0x41` responses on identifier `0x7E8`, the PID in byte 2 acting
//! as the multiplexor, the value in the bytes after it. Nothing in it was derived
//! from this crate's output, which is the property that makes the checks here
//! worth running: if the bit extraction or the scaling were wrong, the numbers
//! would not agree with what the specification says those bytes mean.
//!
//! The strongest check is `run_time_tracks_the_logs_own_clock`. PID `0x1F` is
//! seconds since the engine started, as a 16-bit big-endian value. The file also
//! carries its own timestamps, in a channel this crate has read since long before
//! any of this existed. Those two have to advance together — and they can only do
//! so if the Motorola bit order, the multiplexor selection and the scaling are all
//! right at once. No fixture supplies that agreement; the car did.
//!
//! The corpus is not checked in, so these tests skip when it is absent.

#![cfg(feature = "dbc")]

use falcon_mdf::CanDatabase;
use falcon_mdf::Mf4File;
use std::path::Path;

const OBD2_LOG: &str =
    "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4";

/// Identifier of the first ECU's response, fixed by ISO 15765.
const RESPONSE_ID: u32 = 0x7E8;

/// A database for the PIDs this log actually contains.
///
/// Bit positions follow the DBC convention for big-endian signals: `23` is the
/// most significant bit of byte 2, `31` that of byte 3. The multiplexor values
/// are the PIDs in decimal — `m12` is PID `0x0C`.
const OBD2_DBC: &str = r#"VERSION "1"
NS_ :
BS_:
BU_: Tester ECU
BO_ 2024 OBD2: 8 ECU
 SG_ PID M : 23|8@0+ (1,0) [0|255] "" Tester
 SG_ CoolantTemperature m5 : 31|8@0+ (1,-40) [-40|215] "degC" Tester
 SG_ IntakeManifoldPressure m11 : 31|8@0+ (1,0) [0|255] "kPa" Tester
 SG_ EngineSpeed m12 : 31|16@0+ (0.25,0) [0|16383.75] "rpm" Tester
 SG_ VehicleSpeed m13 : 31|8@0+ (1,0) [0|255] "km/h" Tester
 SG_ ThrottlePosition m17 : 31|8@0+ (0.3921568627,0) [0|100] "%" Tester
 SG_ RunTimeSinceStart m31 : 31|16@0+ (1,0) [0|65535] "s" Tester
"#;

fn database() -> CanDatabase {
    CanDatabase::from_dbc(OBD2_DBC.as_bytes()).expect("the OBD2 database must parse")
}

fn skip() -> bool {
    if Path::new(OBD2_LOG).exists() {
        return false;
    }
    eprintln!("SKIP: {OBD2_LOG} is absent");
    true
}

/// Every response frame, as `(timestamp, payload)`.
fn responses() -> Vec<(f64, Vec<u8>)> {
    let file = Mf4File::open(OBD2_LOG).unwrap();
    let mut out = Vec::new();
    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            if frame.id == RESPONSE_ID {
                out.push((frame.timestamp, frame.data.to_vec()));
            }
        }
    }
    out
}

#[test]
fn the_database_names_the_message() {
    let db = database();
    assert_eq!(db.message_name(RESPONSE_ID), Some("OBD2"));
    assert_eq!(db.message_name(0x7DF), None, "requests are not in this DBC");
}

/// A frame whose identifier the database does not cover decodes to nothing,
/// rather than to a message that happens to sit nearby.
#[test]
fn an_unknown_identifier_decodes_to_nothing() {
    let db = database();
    assert!(db.decode(0x123, &[0xFF; 8]).is_empty());
    assert!(db
        .decode(0x7DF, &[0x02, 0x01, 0x0C, 0, 0, 0, 0, 0])
        .is_empty());
}

/// Multiplexing must select exactly the one signal the PID names — not all of
/// them, and not none.
#[test]
fn the_pid_selects_one_signal() {
    if skip() {
        return;
    }
    let db = database();
    let mut seen = std::collections::BTreeSet::new();

    for (_, payload) in responses() {
        let decoded = db.decode(RESPONSE_ID, &payload);
        let named: Vec<&str> = decoded
            .iter()
            .map(|s| s.name)
            .filter(|name| *name != "PID")
            .collect();

        assert!(
            named.len() <= 1,
            "PID {:#04X} selected {named:?}",
            payload[2]
        );
        if let Some(name) = named.first() {
            seen.insert(name.to_string());
        }
        // The multiplexor itself is always decoded, and must equal the PID byte.
        let pid = decoded.iter().find(|s| s.name == "PID").expect("no PID");
        assert_eq!(pid.value, f64::from(payload[2]), "the multiplexor's value");
    }

    assert_eq!(
        seen.len(),
        6,
        "expected all six PIDs the log carries, saw {seen:?}"
    );
}

/// Seconds since engine start, decoded from a 16-bit big-endian field, must
/// advance in step with the file's own timestamps.
///
/// This is the check that cannot be satisfied by a wrong decoder: the two
/// quantities come from different places — one from a DBC-scaled payload field,
/// one from the MF4 master channel — and only agree if the payload field is being
/// read correctly.
#[test]
fn run_time_tracks_the_logs_own_clock() {
    if skip() {
        return;
    }
    let db = database();

    let mut samples: Vec<(f64, f64)> = Vec::new();
    for (timestamp, payload) in responses() {
        for signal in db.decode(RESPONSE_ID, &payload) {
            if signal.name == "RunTimeSinceStart" {
                samples.push((timestamp, signal.value));
            }
        }
    }

    assert!(
        samples.len() > 1000,
        "expected a run of run-time samples, got {}",
        samples.len()
    );

    let (first_t, first_v) = samples[0];
    let (last_t, last_v) = samples[samples.len() - 1];
    let elapsed_clock = last_t - first_t;
    let elapsed_engine = last_v - first_v;

    assert!(
        elapsed_clock > 60.0,
        "the log is too short to compare clocks: {elapsed_clock} s"
    );
    // Both count seconds, but not the same seconds: the engine's counter has
    // one-second resolution, is sampled a request-response round trip late, and
    // stops counting when the engine does — this car brakes to 6 km/h, so a
    // start-stop pause is likely among the 5 s of the 556 observed. Two percent
    // absorbs all of that and still leaves the check decisive: reading the field
    // in the wrong byte order turns 167 seconds into 42752.
    let drift = (elapsed_engine - elapsed_clock).abs();
    assert!(
        drift < 0.02 * elapsed_clock,
        "engine run time advanced {elapsed_engine} s while the log advanced \
         {elapsed_clock} s — {drift} s apart"
    );

    // And it must be monotonic, as a counter is.
    for pair in samples.windows(2) {
        assert!(
            pair[1].1 >= pair[0].1,
            "run time went backwards: {} then {}",
            pair[0].1,
            pair[1].1
        );
    }
}

/// The specification gives each PID an arithmetic meaning. Decoding must produce
/// exactly that, computed here from the raw bytes rather than taken from the
/// decoder.
#[test]
fn decoded_values_match_the_published_formulas() {
    if skip() {
        return;
    }
    let db = database();
    let mut checked = 0usize;

    for (_, payload) in responses() {
        let a = f64::from(payload[3]);
        let b = f64::from(payload[4]);

        // SAE J1979: the formula for each PID, byte A being payload[3].
        let expected: Option<(&str, f64)> = match payload[2] {
            0x05 => Some(("CoolantTemperature", a - 40.0)),
            0x0B => Some(("IntakeManifoldPressure", a)),
            0x0C => Some(("EngineSpeed", (a * 256.0 + b) / 4.0)),
            0x0D => Some(("VehicleSpeed", a)),
            0x1F => Some(("RunTimeSinceStart", a * 256.0 + b)),
            _ => None,
        };
        let Some((name, want)) = expected else {
            continue;
        };

        let decoded = db.decode(RESPONSE_ID, &payload);
        let got = decoded
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("PID {:#04X} did not decode {name}", payload[2]));
        assert!(
            (got.value - want).abs() < 1e-9,
            "{name}: decoded {} from {:02X?}, the formula gives {want}",
            got.value,
            &payload[..5]
        );
        checked += 1;
    }

    assert!(checked > 5000, "only {checked} values were checked");
    eprintln!("checked {checked} decoded values against J1979 formulas");
}

/// Physical plausibility, which catches a decoder that is self-consistent but
/// reading the wrong bits: a car's coolant does not sit at 200 °C, and an idling
/// engine does not turn at 60000 rpm.
#[test]
fn decoded_values_are_physically_plausible() {
    if skip() {
        return;
    }
    let db = database();
    let mut ranges: std::collections::BTreeMap<String, (f64, f64)> = Default::default();

    for (_, payload) in responses() {
        for signal in db.decode(RESPONSE_ID, &payload) {
            let entry = ranges
                .entry(signal.name.to_string())
                .or_insert((f64::INFINITY, f64::NEG_INFINITY));
            entry.0 = entry.0.min(signal.value);
            entry.1 = entry.1.max(signal.value);
        }
    }

    let (rpm_min, rpm_max) = ranges["EngineSpeed"];
    assert!(
        rpm_min >= 0.0 && rpm_max < 8000.0,
        "engine speed ranged {rpm_min}..{rpm_max} rpm"
    );
    assert!(rpm_max > 600.0, "the engine never left idle: {rpm_max} rpm");

    let (coolant_min, coolant_max) = ranges["CoolantTemperature"];
    assert!(
        coolant_min > -40.0 && coolant_max < 130.0,
        "coolant ranged {coolant_min}..{coolant_max} °C"
    );

    let (speed_min, speed_max) = ranges["VehicleSpeed"];
    assert!(
        speed_min >= 0.0 && speed_max <= 255.0,
        "vehicle speed ranged {speed_min}..{speed_max} km/h"
    );

    let (throttle_min, throttle_max) = ranges["ThrottlePosition"];
    assert!(
        throttle_min >= 0.0 && throttle_max <= 100.0,
        "throttle ranged {throttle_min}..{throttle_max} %"
    );

    eprintln!("decoded ranges: {ranges:?}");
}
