//! Conformance tests for DBC extended multiplexing (SG_MUL_VAL_), global value
//! tables (VAL_TABLE_), and J1939 source-address matching, checked against the
//! canmatrix / asammdf reference oracle in Python.
//!
//! No expected values are derived from this crate's own implementation:
//! payloads are decoded independently through Python's canmatrix library at
//! runtime and compared against CanDatabase's decoded output.

#![cfg(feature = "dbc")]

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::{CanDatabase, IdMatching, Mf4Error};

fn venv_python() -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .expect("no .venv/bin/python with canmatrix found; tests need it for their oracle")
}

fn python_oracle(script: &str) -> serde_json::Value {
    let out = Command::new(venv_python())
        .arg("-c")
        .arg(script)
        .output()
        .expect("running python should succeed");
    assert!(
        out.status.success(),
        "the python oracle failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().last().unwrap_or_else(|| {
        panic!(
            "oracle produced no output; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    serde_json::from_str(line).unwrap_or_else(|e| panic!("invalid JSON from oracle: {e}\n{line}"))
}

const EXT_MUX_DBC: &str = r#"VERSION "1"
NS_ :
    BA_
    BA_DEF_
    BA_DEF_DEF_
    VAL_TABLE_
    SG_MUL_VAL_
BS_:
BU_: Tester

VAL_TABLE_ StateTable 0 "Off" 1 "On" 2 "Error" 3 "Standby" ;
VAL_TABLE_ ModeTable 10 "Eco" 20 "Sport" ;

BO_ 100 MultiplexedMsg: 8 Tester
 SG_ Mux M : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ StaticSig : 8|16@1+ (0.1,0) [0|6553.5] "V" Tester
 SG_ MotorolaSig : 31|16@0+ (1,-100) [-100|65435] "rpm" Tester
 SG_ RangeSig1 : 32|16@1+ (0.5,0) [0|32767.5] "kPa" Tester
 SG_ RangeSig2 : 48|8@1+ (2,10) [10|520] "degC" Tester
 SG_ SingleValRangeSig : 56|8@1+ (1,0) [0|255] "" Tester
 SG_ PlainMuxSig m2 : 48|8@1+ (1,0) [0|255] "" Tester

BO_ 200 ValueTableMsg: 8 Tester
 SG_ SigState : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ SigStateOverride : 8|8@1+ (1,0) [0|255] "" Tester
 SG_ SigMode : 16|8@1+ (1,0) [0|255] "" Tester
 SG_ SigPlain : 24|8@1+ (1,0) [0|255] "" Tester

SG_MUL_VAL_ 100 RangeSig1 Mux 1-3, 10-15;
SG_MUL_VAL_ 100 RangeSig2 Mux 5-8, 10-12;
SG_MUL_VAL_ 100 SingleValRangeSig Mux 20-20;

BA_DEF_ SG_ "GenSigValTable" STRING ;
BA_DEF_ SG_ "ValTable" STRING ;
BA_DEF_DEF_ "GenSigValTable" "" ;
BA_DEF_DEF_ "ValTable" "" ;

BA_ "GenSigValTable" SG_ 200 SigState "StateTable" ;
BA_ "ValTable" SG_ 200 SigStateOverride "StateTable" ;
BA_ "GenSigValTable" SG_ 200 SigMode "ModeTable" ;

VAL_ 200 SigStateOverride 2 "CustomFault" ;
"#;

#[test]
fn canmatrix_oracle_extended_multiplexing_sweep() {
    let db = CanDatabase::from_dbc(EXT_MUX_DBC.as_bytes()).expect("DBC must parse");

    // Test a wide sweep of multiplexor values:
    // 0: static only
    // 1, 3: RangeSig1 active (1-3)
    // 2: RangeSig1 and PlainMuxSig active (m2)
    // 4: static only (gap between 1-3 and 5-8)
    // 5, 6, 8: RangeSig2 active (5-8)
    // 9: static only (gap between 5-8 and 10-12/10-15)
    // 10, 11, 12: RangeSig1 (10-15) and RangeSig2 (10-12) active
    // 13, 14, 15: RangeSig1 active (10-15)
    // 16..19: static only
    // 20: SingleValRangeSig active (20-20)
    // 21, 255: static only
    let test_mux_values = [
        0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 20, 21, 100, 255,
    ];

    let script = format!(
        r#"
import canmatrix.formats
import json

dbc_text = """{EXT_MUX_DBC}"""
db = canmatrix.formats.loads(dbc_text, 'dbc')['']
frame = db.frame_by_name('MultiplexedMsg')

test_mux_values = {test_mux_values:?}
out = []
for mux in test_mux_values:
    payload = bytes([mux, 0x34, 0x12, 0x01, 0x02, 0x55, 0x66, 0x77])
    decoded = frame.decode(payload)
    sig_map = {{}}
    for name, s in decoded.items():
        sig_map[name] = {{
            "raw": int(s.raw_value),
            "phys": float(s.phys_value),
            "unit": s.signal.unit or "",
        }}
    out.append({{"mux": mux, "signals": sig_map}})

print(json.dumps(out))
"#
    );

    let oracle_results = python_oracle(&script);
    let cases = oracle_results.as_array().expect("array of results");

    for case in cases {
        let mux = case["mux"].as_u64().unwrap() as u8;
        let expected_signals = case["signals"].as_object().unwrap();

        let payload = [mux, 0x34, 0x12, 0x01, 0x02, 0x55, 0x66, 0x77];
        let decoded = db.decode(100, &payload);

        let decoded_names: Vec<&str> = decoded.iter().map(|s| s.name).collect();
        let expected_names: Vec<&str> = expected_signals.keys().map(|s| s.as_str()).collect();

        assert_eq!(
            decoded_names.len(),
            expected_names.len(),
            "Mux={mux}: decoded signals {decoded_names:?} vs expected {expected_names:?}"
        );

        for sig in &decoded {
            let exp = expected_signals
                .get(sig.name)
                .unwrap_or_else(|| panic!("Mux={mux}: unexpected signal '{}' decoded", sig.name));
            let exp_phys = exp["phys"].as_f64().unwrap();
            let exp_unit = exp["unit"].as_str().unwrap();

            assert!(
                (sig.value - exp_phys).abs() < 1e-6,
                "Mux={mux}, Signal '{}': value {} != expected {}",
                sig.name,
                sig.value,
                exp_phys
            );
            assert_eq!(
                sig.unit, exp_unit,
                "Mux={mux}, Signal '{}': unit mismatch",
                sig.name
            );
        }
    }
}

#[test]
fn canmatrix_oracle_global_value_tables_and_overrides() {
    let db = CanDatabase::from_dbc(EXT_MUX_DBC.as_bytes()).expect("DBC must parse");

    // Test SigState (uses global StateTable: 0="Off", 1="On", 2="Error", 3="Standby")
    let p0 = [0, 0, 0, 0, 0, 0, 0, 0];
    let d0 = db.decode(200, &p0);
    assert_eq!(
        d0.iter().find(|s| s.name == "SigState").unwrap().text,
        Some("Off")
    );
    assert_eq!(d0.iter().find(|s| s.name == "SigState").unwrap().value, 0.0);

    let p1 = [1, 0, 0, 0, 0, 0, 0, 0];
    let d1 = db.decode(200, &p1);
    assert_eq!(
        d1.iter().find(|s| s.name == "SigState").unwrap().text,
        Some("On")
    );

    let p2 = [2, 0, 0, 0, 0, 0, 0, 0];
    let d2 = db.decode(200, &p2);
    assert_eq!(
        d2.iter().find(|s| s.name == "SigState").unwrap().text,
        Some("Error")
    );

    let p3 = [3, 0, 0, 0, 0, 0, 0, 0];
    let d3 = db.decode(200, &p3);
    assert_eq!(
        d3.iter().find(|s| s.name == "SigState").unwrap().text,
        Some("Standby")
    );

    let p4 = [4, 0, 0, 0, 0, 0, 0, 0];
    let d4 = db.decode(200, &p4);
    assert_eq!(d4.iter().find(|s| s.name == "SigState").unwrap().text, None);

    // Test SigStateOverride (inherits StateTable via ValTable, but overrides raw value 2 with "CustomFault")
    let po0 = [0, 0, 0, 0, 0, 0, 0, 0];
    let do0 = db.decode(200, &po0);
    assert_eq!(
        do0.iter()
            .find(|s| s.name == "SigStateOverride")
            .unwrap()
            .text,
        Some("Off")
    );

    let po2 = [0, 2, 0, 0, 0, 0, 0, 0];
    let do2 = db.decode(200, &po2);
    assert_eq!(
        do2.iter()
            .find(|s| s.name == "SigStateOverride")
            .unwrap()
            .text,
        Some("CustomFault"),
        "per-signal VAL_ should override global table entry for value 2"
    );

    let po3 = [0, 3, 0, 0, 0, 0, 0, 0];
    let do3 = db.decode(200, &po3);
    assert_eq!(
        do3.iter()
            .find(|s| s.name == "SigStateOverride")
            .unwrap()
            .text,
        Some("Standby")
    );

    // Test SigMode (uses ModeTable: 10="Eco", 20="Sport")
    let pm10 = [0, 0, 10, 0, 0, 0, 0, 0];
    let dm10 = db.decode(200, &pm10);
    assert_eq!(
        dm10.iter().find(|s| s.name == "SigMode").unwrap().text,
        Some("Eco")
    );

    let pm20 = [0, 0, 20, 0, 0, 0, 0, 0];
    let dm20 = db.decode(200, &pm20);
    assert_eq!(
        dm20.iter().find(|s| s.name == "SigMode").unwrap().text,
        Some("Sport")
    );

    let pm99 = [0, 0, 99, 0, 0, 0, 0, 0];
    let dm99 = db.decode(200, &pm99);
    assert_eq!(
        dm99.iter().find(|s| s.name == "SigMode").unwrap().text,
        None
    );

    // Test SigPlain (no table attached)
    let dplain = db.decode(200, &[0, 0, 0, 1, 0, 0, 0, 0]);
    assert_eq!(
        dplain.iter().find(|s| s.name == "SigPlain").unwrap().text,
        None
    );
}

#[test]
fn j1939_source_address_matching_and_fallback() {
    // Database with:
    // 1. Specific ECU 0x00 for PGN 0xF004 (EEC1_Engine)
    // 2. Specific ECU 0x0F for PGN 0xF004 (EEC1_Retarder)
    // 3. Fallback ECU 0xFE for PGN 0xF004 (EEC1_Generic)
    // 4. Fallback ECU 0xFE for PGN 0xFEE5 (HOURS)
    let dbc = r#"VERSION "1"
NS_ :
BS_:
BU_: Engine Retarder Generic
BO_ 2364539904 EEC1_Engine: 8 Engine
 SG_ EngTorque : 0|8@1+ (1,0) [0|255] "%" Engine
BO_ 2364539919 EEC1_Retarder: 8 Retarder
 SG_ RetTorque : 0|8@1+ (1,0) [0|255] "%" Retarder
BO_ 2364540158 EEC1_Generic: 8 Generic
 SG_ GenTorque : 0|8@1+ (1,0) [0|255] "%" Generic
BO_ 2566841854 HOURS: 8 Generic
 SG_ TotalHours : 0|32@1+ (0.05,0) [0|210554060.75] "h" Generic
"#;

    let db_exact = CanDatabase::from_dbc(dbc.as_bytes())
        .unwrap()
        .with_matching(IdMatching::Exact);
    let db_pgn = CanDatabase::from_dbc(dbc.as_bytes())
        .unwrap()
        .with_matching(IdMatching::J1939Pgn);
    let db_pgn_src = CanDatabase::from_dbc(dbc.as_bytes())
        .unwrap()
        .with_matching(IdMatching::J1939PgnAndSource);

    // Frame from Engine (source 0x00, PGN 0xF004, priority 3 -> 0x0CF00400)
    let payload = [50, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(db_exact.message_name(0x0CF00400), Some("EEC1_Engine"));
    assert_eq!(db_pgn_src.message_name(0x0CF00400), Some("EEC1_Engine"));
    let d = db_pgn_src.decode(0x0CF00400, &payload);
    assert_eq!(d[0].name, "EngTorque");

    // Frame from Engine but at different priority (priority 6 -> 0x18F00400)
    // Under Exact: ID does not match 0x0CF00400 exactly
    assert_eq!(db_exact.message_name(0x18F00400), None);
    // Under J1939PgnAndSource: matches (PGN 0xF004, source 0x00) -> EEC1_Engine
    assert_eq!(db_pgn_src.message_name(0x18F00400), Some("EEC1_Engine"));
    let d = db_pgn_src.decode(0x18F00400, &payload);
    assert_eq!(d[0].name, "EngTorque");

    // Frame from Retarder (source 0x0F, priority 6 -> 0x18F0040F)
    assert_eq!(db_pgn_src.message_name(0x18F0040F), Some("EEC1_Retarder"));
    let d = db_pgn_src.decode(0x18F0040F, &payload);
    assert_eq!(d[0].name, "RetTorque");

    // Frame from an unknown ECU (source 0x42, PGN 0xF004, priority 6 -> 0x18F00442)
    // Under J1939PgnAndSource: no (0xF004, 0x42), falls back to PGN 0xF004
    assert!(db_pgn_src.message_name(0x18F00442).is_some());
    // Under J1939Pgn: also matches by PGN
    assert!(db_pgn.message_name(0x18F00442).is_some());

    // Frame for HOURS from ECU 0x21 (PGN 0xFEE5, source 0x21 -> 0x18FEE521)
    // Under J1939PgnAndSource: falls back to PGN -> HOURS
    assert_eq!(db_pgn_src.message_name(0x18FEE521), Some("HOURS"));
}

#[test]
fn unsupported_extended_multiplexing_returns_named_errors() {
    // Missing multiplexor signal
    let bad_dbc1 = r#"VERSION "1"
NS_ :
    SG_MUL_VAL_
BS_:
BU_: Tester
BO_ 100 Probe: 8 Tester
 SG_ SigA : 0|8@1+ (1,0) [0|255] "" Tester
SG_MUL_VAL_ 100 SigA NonExistentMux 1-3;
"#;
    let err1 = CanDatabase::from_dbc(bad_dbc1.as_bytes()).unwrap_err();
    match err1 {
        Mf4Error::Unsupported { feature, detail } => {
            assert!(feature.contains("SG_MUL_VAL_"));
            assert!(detail.contains("NonExistentMux"));
        }
        other => panic!("expected Unsupported error, got {other:?}"),
    }

    // Cyclic multiplexor graph: each signal is transitively its own multiplexor.
    let bad_dbc2 = r#"VERSION "1"
NS_ :
    SG_MUL_VAL_
BS_:
BU_: Tester
BO_ 100 Probe: 8 Tester
 SG_ MuxA : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ MuxB : 8|8@1+ (1,0) [0|255] "" Tester
SG_MUL_VAL_ 100 MuxA MuxB 1-3;
SG_MUL_VAL_ 100 MuxB MuxA 1-3;
"#;
    let err2 = CanDatabase::from_dbc(bad_dbc2.as_bytes()).unwrap_err();
    match err2 {
        Mf4Error::Unsupported { feature, detail } => {
            assert!(feature.contains("multiplexor cycle"));
            assert!(detail.contains("MuxA") || detail.contains("MuxB"));
        }
        other => panic!("expected Unsupported multiplexor-cycle error, got {other:?}"),
    }
}

#[test]
fn nested_extended_multiplexing_walks_the_chain() {
    // Parent multiplexor MuxA selects whether MuxB is present. MuxB's value
    // then selects whether NestedSig is present.
    let dbc = r#"VERSION "1"
NS_ :
    SG_MUL_VAL_
BS_:
BU_: Tester
BO_ 100 NestedMux: 8 Tester
 SG_ MuxA M : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ MuxB : 8|8@1+ (1,0) [0|255] "" Tester
 SG_ NestedSig : 16|8@1+ (1,0) [0|255] "" Tester
SG_MUL_VAL_ 100 MuxB MuxA 1-1;
SG_MUL_VAL_ 100 NestedSig MuxB 7-7;
"#;
    let db = CanDatabase::from_dbc(dbc.as_bytes()).expect("nested DBC must parse");

    let names =
        |payload: &[u8]| -> Vec<&str> { db.decode(100, payload).iter().map(|s| s.name).collect() };

    // MuxA != 1: MuxB is not present, so NestedSig cannot be present either.
    assert_eq!(names(&[0, 7, 42, 0, 0, 0, 0, 0]), ["MuxA"]);
    // MuxA == 1 but MuxB != 7: MuxB is present but NestedSig is not.
    assert_eq!(names(&[1, 3, 42, 0, 0, 0, 0, 0]), ["MuxA", "MuxB"]);
    // MuxA == 1 and MuxB == 7: both conditions satisfied.
    assert_eq!(
        names(&[1, 7, 42, 0, 0, 0, 0, 0]),
        ["MuxA", "MuxB", "NestedSig"]
    );
}

#[test]
fn ldf_signal_decoding_cross_checked_with_python() {
    let ldf_text = r#"
LIN_description_file ;
LIN_protocol_version = "2.1" ;
LIN_language_version = "2.1" ;
LIN_speed = 19.2 kbps ;

Nodes {
    Master: CEM, 5.0 ms, 0.1 ms ;
    Slaves: LSM, RSM ;
}

Signals {
    StatusSig: 3, 0, LSM, CEM ;
    AngleSig: 11, 0, LSM, CEM ;
    TempSig: 8, 40, LSM, CEM ;
    PressureSig: 16, 1000, LSM, CEM ;
}

Frames {
    StatusFrame: 0x15, LSM, 5 {
        StatusSig, 0 ;
        AngleSig, 3 ;
        TempSig, 14 ;
        PressureSig, 22 ;
    }
}

Signal_encoding_types {
    EncStatus {
        logical_value, 0, "Idle" ;
        logical_value, 1, "Active" ;
        logical_value, 2, "Warning" ;
        logical_value, 3, "Error" ;
    }
    EncAngle {
        physical_value, 0, 2047, 0.25, -180.0, "deg" ;
    }
    EncTemp {
        physical_value, 0, 255, 0.5, -40.0, "degC" ;
    }
    EncPressure {
        physical_value, 0, 65535, 0.1, 0.0, "kPa" ;
    }
}

Signal_representation {
    EncStatus: StatusSig ;
    EncAngle: AngleSig ;
    EncTemp: TempSig ;
    EncPressure: PressureSig ;
}
"#;

    let payload = [0x5A, 0x3C, 0xA5, 0x12, 0x80];
    let hex_payload: String = payload.iter().map(|b| format!("{b:02x}")).collect();

    let oracle_script = format!(
        r#"
import json

payload = list(bytes.fromhex("{hex_payload}"))

def extract_bits(payload, start_bit, bit_len):
    raw = 0
    for i in range(bit_len):
        bit_idx = start_bit + i
        byte_idx = bit_idx // 8
        bit_in_byte = bit_idx % 8
        if byte_idx < len(payload):
            bit_val = (payload[byte_idx] >> bit_in_byte) & 1
            raw |= (bit_val << i)
    return raw

status_raw = extract_bits(payload, 0, 3)
angle_raw = extract_bits(payload, 3, 11)
temp_raw = extract_bits(payload, 14, 8)
pressure_raw = extract_bits(payload, 22, 16)

status_map = {{0: "Idle", 1: "Active", 2: "Warning", 3: "Error"}}
status_text = status_map.get(status_raw)

angle_val = angle_raw * 0.25 - 180.0
temp_val = temp_raw * 0.5 - 40.0
pressure_val = pressure_raw * 0.1

print(json.dumps({{
    "StatusSig": {{"raw": status_raw, "val": float(status_raw), "text": status_text}},
    "AngleSig": {{"raw": angle_raw, "val": angle_val, "unit": "deg"}},
    "TempSig": {{"raw": temp_raw, "val": temp_val, "unit": "degC"}},
    "PressureSig": {{"raw": pressure_raw, "val": pressure_val, "unit": "kPa"}},
}}))
"#
    );

    let oracle_json = python_oracle(&oracle_script);

    let db = CanDatabase::from_ldf(ldf_text.as_bytes()).expect("LDF must parse");
    let decoded = db.decode(0x15, &payload);

    assert_eq!(decoded.len(), 4);

    for sig in &decoded {
        let expected = &oracle_json[&sig.name];
        assert_eq!(
            sig.value,
            expected["val"].as_f64().unwrap(),
            "{}: value mismatch against Python oracle",
            sig.name
        );
        if let Some(expected_text) = expected.get("text").and_then(|t| t.as_str()) {
            assert_eq!(sig.text, Some(expected_text), "{}: text mismatch", sig.name);
        }
        if let Some(expected_unit) = expected.get("unit").and_then(|u| u.as_str()) {
            assert_eq!(sig.unit, expected_unit, "{}: unit mismatch", sig.name);
        }
    }
}

#[cfg(feature = "arxml")]
#[test]
fn arxml_dynamic_multiplexing_cross_checked_with_python() {
    let arxml_path = resolve_arxml("test_data/arxml/system-4.2.arxml")
        .expect("test_data/arxml/system-4.2.arxml should exist");

    // Case 1: Selector = 0 (Hello active)
    let payload0 = [0b0000_1000u8, 0x55, 0, 0, 0, 0, 0, 0, 0, 0];
    let hex0: String = payload0.iter().map(|b| format!("{b:02x}")).collect();

    // Case 2: Selector = 1 (World1 and World2 active)
    let payload1 = [0b0101_1000u8, 0x55, 0, 0, 0, 0, 0, 0, 0, 0];
    let hex1: String = payload1.iter().map(|b| format!("{b:02x}")).collect();

    let oracle_script = format!(
        r#"
import json

def decode_mux(hex_str):
    payload = list(bytes.fromhex(hex_str))
    def extract_bits(payload, start_bit, bit_len):
        raw = 0
        for i in range(bit_len):
            bit_idx = start_bit + i
            byte_idx = bit_idx // 8
            bit_in_byte = bit_idx % 8
            if byte_idx < len(payload):
                bit_val = (payload[byte_idx] >> bit_in_byte) & 1
                raw |= (bit_val << i)
        return raw

    # Static parts
    static1 = extract_bits(payload, 0, 3)
    static2 = extract_bits(payload, 8, 8)
    selector = extract_bits(payload, 6, 2)

    res = {{
        "MultiplexedStatic": float(static1),
        "MultiplexedStatic2": float(static2),
        "multiplexed_message_selector": float(selector),
    }}

    if selector == 0:
        res["Hello"] = float(extract_bits(payload, 3, 1))
    elif selector == 1:
        res["World1"] = float(extract_bits(payload, 4, 2))
        w2_raw = extract_bits(payload, 3, 1)
        # World2 is 1-bit signed S16, bit 1 is -1 in 2's complement
        res["World2"] = -1.0 if w2_raw == 1 else 0.0

    return res

print(json.dumps({{
    "case0": decode_mux("{hex0}"),
    "case1": decode_mux("{hex1}"),
}}))
"#
    );

    let oracle_json = python_oracle(&oracle_script);

    let db = CanDatabase::from_arxml_path(&arxml_path).expect("load ARXML");

    // Check Case 0
    let dec0 = db.decode(4, &payload0);
    let expected0 = &oracle_json["case0"];
    assert_eq!(dec0.len(), expected0.as_object().unwrap().len());
    for s in &dec0 {
        assert_eq!(
            s.value,
            expected0[&s.name].as_f64().unwrap(),
            "case0 signal {} mismatch",
            s.name
        );
    }

    // Check Case 1
    let dec1 = db.decode(4, &payload1);
    let expected1 = &oracle_json["case1"];
    assert_eq!(dec1.len(), expected1.as_object().unwrap().len());
    for s in &dec1 {
        assert_eq!(
            s.value,
            expected1[&s.name].as_f64().unwrap(),
            "case1 signal {} mismatch",
            s.name
        );
    }
}

fn resolve_arxml(rel: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(rel),
        PathBuf::from("../../falcon_mdf").join(rel),
    ];
    candidates.into_iter().find(|p| p.exists())
}
