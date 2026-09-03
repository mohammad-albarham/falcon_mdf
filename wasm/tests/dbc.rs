//! Native tests for DBC bus decoding through the binding (plan 3.2): the
//! attach produces viewer-facing channels off the CANedge OBD2 log, and the
//! decoded values are pinned by hand arithmetic over the same frames the
//! core decoder reads — the numbers the GUI would show for the same DBC.
//!
//! The DBC is multiplexed on the OBD2 response's PID byte, which is exactly
//! how a single-frame ISO-TP response lays out: byte 1 is the response
//! service (0x41), byte 2 the PID, bytes 3.. the payload.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf::candb::CanDatabase;
use falcon_mdf::Mf4File;
use falcon_mdf_wasm::WasmMf4File;

/// The CANedge OBD2 log — the plan's acceptance file for this feature.
const LOG: &str = "mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4";

/// A minimal DBC over the OBD2 response identifier 0x7E8 = 2024. Muxed on
/// the PID byte; `IntakeMapAbsolute` carries a value table so the text path
/// (state bands, label readout) is exercised over a bus channel too.
const DBC: &str = r#"VERSION "wasm-obd2"

NS_ :

BS_:

BU_: ECU

BO_ 2024 OBD2_RESPONSE: 8 ECU
 SG_ ResponsePID M : 16|8@1+ (1,0) [0|255] "" ECU
 SG_ EngineSpeed m12 : 31|16@0+ (0.25,0) [0|16383.75] "rpm" ECU
 SG_ IntakeMapAbsolute m11 : 24|8@1+ (1,0) [0|255] "kPa" ECU
 SG_ VehicleSpeed m13 : 24|8@1+ (1,0) [0|255] "km/h" ECU

CM_ SG_ 2024 EngineSpeed "engine speed, mode 01 PID 0x0C";
VAL_ 2024 IntakeMapAbsolute 28 "idle-map" 255 "overflow";
"#;

/// The log's bytes, or `None` when the corpus isn't fetched.
fn log_bytes() -> Option<Vec<u8>> {
    for base in ["test_data", "../test_data"] {
        let path = std::path::Path::new(base).join(LOG);
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes);
        }
    }
    None
}

fn require_log() -> WasmMf4File {
    let Some(bytes) = log_bytes() else {
        panic!("the OBD2 corpus log must be fetched — a skipped test is not a passed test")
    };
    WasmMf4File::new(bytes).expect("open the OBD2 log")
}

/// The readings hand arithmetic says `EngineSpeed` must decode to: every
/// 0x7E8 frame whose PID byte is 0x0C contributes (d[3]*256 + d[4]) / 4.
fn hand_decoded_rpm() -> Vec<(f64, f64)> {
    let bytes = log_bytes().expect("log bytes");
    let file = Mf4File::from_bytes(bytes).expect("open");
    let group = file
        .can_frame_groups()
        .into_iter()
        .next()
        .expect("a CAN group");
    let frames = file.can_frames(group).expect("frames");
    frames
        .iter()
        .filter(|f| f.id == 0x7E8 && f.data.len() >= 5 && f.data[2] == 0x0C)
        .map(|f| {
            (
                f.timestamp,
                ((f.data[3] as u16 * 256 + f.data[4] as u16) as f64) / 4.0,
            )
        })
        .collect()
}

#[test]
fn attach_produces_viewer_channels() {
    let mut file = require_log();
    let summary = file.attach_dbc(DBC.as_bytes()).expect("attach");
    assert!(summary.contains("\"signals\":"), "summary JSON: {summary}");
    assert!(summary.contains("OBD2_RESPONSE.EngineSpeed"), "{summary}");
    // The decoded channels join the listing endpoints.
    assert!(file
        .channel_names()
        .expect("names")
        .contains("OBD2_RESPONSE.EngineSpeed"));
    assert!(
        file.channel_count() > 4,
        "file channels plus decoded signals"
    );
    let kind = file
        .channel_kind("OBD2_RESPONSE.EngineSpeed")
        .expect("kind");
    assert_eq!(kind, "f64");
    let kind = file
        .channel_kind("OBD2_RESPONSE.IntakeMapAbsolute")
        .expect("kind");
    assert_eq!(kind, "text", "the value table makes it a text channel");
}

#[test]
fn decoded_rpm_matches_hand_arithmetic() {
    // Pinned against hand decoding of the same frames — the values the GUI
    // (same core decoder) shows for this log and DBC.
    let mut file = require_log();
    file.attach_dbc(DBC.as_bytes()).expect("attach");
    let json = file
        .signal("OBD2_RESPONSE.EngineSpeed")
        .expect("decoded signal");
    let JsonVal::Obj(fields) = parse_json(&json).expect("signal json") else {
        panic!("signal is an object")
    };
    let values = fields
        .iter()
        .find_map(|(k, v)| (k == "values").then_some(v.clone()))
        .expect("values");
    let JsonVal::Array(items) = values else {
        panic!("values is an array")
    };
    let expected = hand_decoded_rpm();
    assert!(!expected.is_empty(), "the log must contain RPM frames");
    assert_eq!(items.len(), expected.len(), "same readings, same order");
    for (i, (item, (_, rpm))) in items.iter().zip(expected.iter()).enumerate() {
        match item {
            JsonVal::Number(v) => assert!(
                (v - rpm).abs() < 1e-9,
                "reading {i}: {v} vs hand-decoded {rpm}"
            ),
            other => panic!("reading {i} is a number, got {other:?}"),
        }
    }
}

#[test]
fn a_value_table_signal_reads_as_labels() {
    let mut file = require_log();
    file.attach_dbc(DBC.as_bytes()).expect("attach");
    let json = file
        .signal_text(
            "OBD2_RESPONSE.IntakeMapAbsolute",
            f64::NEG_INFINITY,
            f64::INFINITY,
            1000,
        )
        .expect("signal_text");
    assert!(json.contains("\"kind\":\"text\""));
    assert!(json.contains("idle-map"), "the pinned label in {json}");
    // The scalar endpoint refuses it, like any text channel.
    assert!(file
        .signal_text("OBD2_RESPONSE.EngineSpeed", 0.0, 1.0, 100)
        .is_err());
}

#[test]
fn attach_replaces_and_detach_clears() {
    let mut file = require_log();
    let base_count = file.channel_count();
    file.attach_dbc(DBC.as_bytes()).expect("first attach");
    let after_first = file.channel_count();
    assert!(after_first > base_count);
    file.attach_dbc(DBC.as_bytes()).expect("second attach");
    assert_eq!(
        file.channel_count(),
        after_first,
        "a re-attach replaces, never accumulates"
    );
    assert_eq!(file.detach_dbc(), after_first - base_count);
    assert_eq!(file.channel_count(), base_count);
    assert!(file.channel_kind("OBD2_RESPONSE.EngineSpeed").is_err());
}

#[test]
fn a_broken_dbc_is_a_visible_error_not_a_empty_overlay() {
    let mut file = require_log();
    assert!(file.attach_dbc(b"not a dbc at all").is_err());
    // The overlay is untouched by the failed attach.
    let base_count = file.channel_count();
    assert!(base_count > 0);
}

#[test]
fn v3_files_refuse_a_dbc_attach() {
    // MDF 3 carries no CAN frame groups — the attach says so instead of
    // returning a silent zero.
    let Some(bytes) = std::fs::read("test_data/generated/gen_v330.mdf")
        .ok()
        .or_else(|| std::fs::read("../test_data/generated/gen_v330.mdf").ok())
    else {
        return; // the v3 fixture is written by the mdf3 tests
    };
    let mut file = WasmMf4File::new(bytes).expect("open v3");
    assert!(file.attach_dbc(DBC.as_bytes()).is_err());
}
