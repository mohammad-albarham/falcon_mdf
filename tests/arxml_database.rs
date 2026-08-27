//! ARXML reading checked against cantools, an independently written reader.
//!
//! The fixture is `system-4.2.arxml` from cantools' own test corpus, and the
//! expectations below are transcribed from cantools' assertions about that same
//! file (`tests/test_database.py::test_system_4_arxml`). That is the same standard
//! `reference.rs` holds measurement decoding to: agreement means two separately
//! written readers extract the same definitions from the same bytes.
//!
//! It earned its keep. Three defects in the traversal were found here and nowhere
//! else, all of them silent:
//!
//! - `PACKING-BYTE-ORDER` and `CAN-ADDRESSING-MODE` are AUTOSAR *enums*, and
//!   `CharacterData::string_value()` returns `None` for an enum rather than
//!   failing. Reading them as text reported every signal little-endian and every
//!   message standard — which byte-swaps `signal1` and misreads `Message2`.
//! - A `SCALE_LINEAR_AND_TEXTTABLE` compu method puts a text-table scale *first*,
//!   with no rational coefficients. Taking the first scale found no factor and
//!   reported `signal6` unscaled, at 1 instead of 0.1.
//! - A unit's `SHORT-NAME` is an identifier and its `DISPLAY-NAME` the symbol;
//!   reading the former gave `wizepoo` where the unit is `wp`.
//!
//! The fixture is not checked in. These tests skip when it is absent, as the rest
//! of the corpus-backed suites do.

#![cfg(feature = "arxml")]

use falcon_mdf::{CanDatabase, SignalDef};
use std::path::PathBuf;

const SYSTEM_ARXML: &str = "test_data/arxml/system-4.2.arxml";

fn arxml_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(SYSTEM_ARXML),
        PathBuf::from("../../falcon_mdf").join(SYSTEM_ARXML),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn database() -> Option<CanDatabase> {
    let Some(path) = arxml_path() else {
        eprintln!("SKIP: {SYSTEM_ARXML} is absent");
        return None;
    };
    Some(CanDatabase::from_arxml_path(path).expect("the ARXML fixture must load"))
}

fn signal<'a>(db: &'a CanDatabase, id: u32, name: &str) -> &'a SignalDef {
    db.message(id)
        .unwrap_or_else(|| panic!("no message {id:#X}"))
        .signals
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("message {id:#X} has no signal '{name}'"))
}

/// cantools counts eight messages in this file, with these identifiers, lengths
/// and addressing modes.
#[test]
fn the_messages_match_cantools() {
    let Some(db) = database() else { return };

    assert_eq!(db.messages().len(), 8, "message count");

    // (name, identifier, extended, length)
    let expected = [
        ("MultiplexedMessage", 4u32, false, 2u64),
        ("Message1", 5, false, 9),
        ("Message2", 6, true, 7),
        ("Message3", 100, false, 6),
        ("Message4", 101, false, 6),
        ("OneToContainThemAll", 102, false, 64),
        ("AlarmStatus", 1001, false, 1),
        ("MessageWithoutPDU", 1002, false, 8),
    ];

    for (name, id, extended, length) in expected {
        let message = db
            .message(id)
            .unwrap_or_else(|| panic!("no message with identifier {id}"));
        assert_eq!(message.name, name, "name of {id}");
        assert_eq!(message.extended, extended, "addressing mode of {name}");
        assert_eq!(message.length, length, "length of {name}");
    }
}

/// Message1's five signals, field for field as cantools reports them.
///
/// This is the message that catches byte order: `signal1` is the file's only
/// big-endian signal, and it sits beside little-endian ones in the same payload.
#[test]
fn message1_signals_match_cantools() {
    let Some(db) = database() else { return };

    assert_eq!(db.message(5).unwrap().signals.len(), 5, "signal count");

    // (name, start, size, big_endian, signed, factor, offset, unit)
    let expected = [
        (
            "message1_SeqCounter",
            0u64,
            16u64,
            false,
            false,
            1.0,
            0.0,
            "",
        ),
        ("message1_CRC", 16, 16, false, false, 1.0, 0.0, ""),
        ("signal6", 32, 1, false, false, 0.1, 0.0, "wp"),
        ("signal1", 36, 3, true, false, 5.0, 0.0, ""),
        ("signal5", 40, 32, false, false, 1.0, 0.0, ""),
    ];

    for (name, start, size, big_endian, signed, factor, offset, unit) in expected {
        let got = signal(&db, 5, name);
        assert_eq!(got.start_bit, start, "{name}: start bit");
        assert_eq!(got.size, size, "{name}: width");
        assert_eq!(got.big_endian, big_endian, "{name}: byte order");
        assert_eq!(got.signed, signed, "{name}: signedness");
        assert_eq!(got.factor, factor, "{name}: factor");
        assert_eq!(got.offset, offset, "{name}: offset");
        assert_eq!(got.unit, unit, "{name}: unit");
    }
}

/// Message2 is the extended-identifier message, and carries the file's signed
/// signal.
#[test]
fn message2_signals_match_cantools() {
    let Some(db) = database() else { return };

    assert_eq!(db.message(6).unwrap().signals.len(), 3, "signal count");

    let signal3 = signal(&db, 6, "signal3");
    assert_eq!((signal3.start_bit, signal3.size), (6, 2));
    assert!(!signal3.big_endian);
    assert!(!signal3.signed);

    let signal2 = signal(&db, 6, "signal2");
    assert_eq!((signal2.start_bit, signal2.size), (18, 11));
    assert!(!signal2.big_endian);
    assert!(signal2.signed, "signal2 is two's complement");

    let signal4 = signal(&db, 6, "signal4");
    assert_eq!((signal4.start_bit, signal4.size), (30, 4));
    assert!(!signal4.signed);
    // Divergence from cantools, which reports no unit here. The file does name
    // one — a UNIT whose display name is literally `NoUnit` — and reporting what
    // the file says is preferred to encoding another reader's special case.
    assert_eq!(signal4.unit, "NoUnit");
}

/// Message4 holds three same-width signals that differ only in how they encode
/// sign, which is exactly where reading signedness off the wrong element shows up.
#[test]
fn message4_distinguishes_signed_encodings() {
    let Some(db) = database() else { return };

    let plain = signal(&db, 101, "signal2");
    let ones_complement = signal(&db, 101, "signal2_1c");
    let sign_magnitude = signal(&db, 101, "signal2_sm");

    for s in [plain, ones_complement, sign_magnitude] {
        assert_eq!(s.size, 11, "{}: width", s.name);
    }
    // Only two's complement is a sign encoding this decoder can apply; the other
    // two are reported unsigned rather than decoded wrongly.
    assert!(plain.signed, "signal2 is two's complement");
    assert!(!ones_complement.signed, "signal2_1c is ones complement");
    assert!(!sign_magnitude.signed, "signal2_sm is sign-magnitude");
}

/// A multiplexed message's dynamic parts are conditionally decoded based on
/// the selector field code, so overlapping signals in the payload are never
/// simultaneously decoded.
#[test]
fn multiplexed_dynamic_parts_decode_by_selector() {
    let Some(db) = database() else { return };

    let message = db.message(4).expect("MultiplexedMessage");
    assert_eq!(message.name, "MultiplexedMessage");

    let signal_names: Vec<&str> = message.signals.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        signal_names,
        vec![
            "MultiplexedStatic",
            "MultiplexedStatic2",
            "multiplexed_message_selector",
            "Hello",
            "World1",
            "World2",
        ]
    );

    // Case 1: Selector = 0 (bits 6..8 = 00)
    // Bit 3 set: Hello = 1
    let mut payload0 = [0u8; 10];
    payload0[0] = 0b0000_1000; // selector = 0, Hello (bit 3) = 1
    payload0[1] = 0x42; // MultiplexedStatic2 = 0x42 (66)

    let decoded0 = db.decode(4, &payload0);
    let names0: Vec<&str> = decoded0.iter().map(|s| s.name).collect();
    assert_eq!(
        names0,
        vec![
            "MultiplexedStatic",
            "MultiplexedStatic2",
            "multiplexed_message_selector",
            "Hello",
        ]
    );
    assert_eq!(
        decoded0.iter().find(|s| s.name == "Hello").unwrap().value,
        1.0
    );
    assert_eq!(
        decoded0
            .iter()
            .find(|s| s.name == "MultiplexedStatic2")
            .unwrap()
            .value,
        66.0
    );

    // Case 2: Selector = 1 (bits 6..8 = 01)
    // Bits 4..6 = 01 (World1 = 1), Bit 3 = 1 (World2 = 1)
    let mut payload1 = [0u8; 10];
    payload1[0] = 0b0101_1000; // selector = 1, World1 = 1, World2 = 1
    payload1[1] = 0x42;

    let decoded1 = db.decode(4, &payload1);
    let names1: Vec<&str> = decoded1.iter().map(|s| s.name).collect();
    assert_eq!(
        names1,
        vec![
            "MultiplexedStatic",
            "MultiplexedStatic2",
            "multiplexed_message_selector",
            "World1",
            "World2",
        ]
    );
    assert_eq!(
        decoded1.iter().find(|s| s.name == "World1").unwrap().value,
        1.0
    );
    assert_eq!(
        decoded1.iter().find(|s| s.name == "World2").unwrap().value,
        -1.0,
        "World2 is 1-bit signed S16, so bit 1 sign-extends to -1"
    );

    // Case 3: Selector = 2 (unhandled dynamic code) -> only static and selector signals decode
    let mut payload2 = [0u8; 10];
    payload2[0] = 0b1000_0000; // selector = 2
    payload2[1] = 0x42;

    let decoded2 = db.decode(4, &payload2);
    let names2: Vec<&str> = decoded2.iter().map(|s| s.name).collect();
    assert_eq!(
        names2,
        vec![
            "MultiplexedStatic",
            "MultiplexedStatic2",
            "multiplexed_message_selector",
        ]
    );
}

/// The decoder works on ARXML definitions exactly as it does on DBC ones, since
/// it is the same decoder. Little- and big-endian signals in one payload is the
/// case worth showing.
#[test]
fn arxml_definitions_decode() {
    let Some(db) = database() else { return };

    // message1_SeqCounter is bits 0..16 little-endian; signal1 is three bits
    // big-endian with its most significant bit at position 36.
    let mut payload = [0u8; 9];
    payload[0] = 0x34;
    payload[1] = 0x12;
    // Byte 4 holds signal6 at bit 32 and signal1 at bits 36..39.
    payload[4] = 0b0101_0001;

    let decoded = db.decode(5, &payload);
    let value = |name: &str| {
        decoded
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} did not decode"))
            .value
    };

    assert_eq!(value("message1_SeqCounter"), f64::from(0x1234));
    assert_eq!(value("signal6"), 0.1, "bit 32 set, scaled by 0.1");

    // signal1 is big-endian with its most significant bit at position 36. In
    // MSB-first numbering that is index 35, and the signal runs upwards from
    // there: 35, 36, 37. Those are bits 4, 3 and 2 of byte 4 (0b0101_0001), so
    // 1, 0, 0 — a raw value of 4, scaled by 5.
    assert_eq!(value("signal1"), 20.0);
}

/// A `TEXTTABLE` compu method populates the signal's value table, and decoding
/// reports the associated text for matching raw values, or None when no entry exists.
#[test]
fn texttable_compu_methods_populate_value_tables_and_decode_text() {
    let Some(db) = database() else { return };

    let sig4 = signal(&db, 6, "signal4");
    assert_eq!(
        sig4.value_table,
        vec![(1, "one".to_string()), (2, "two".to_string())]
    );

    let selector = signal(&db, 4, "multiplexed_message_selector");
    assert_eq!(
        selector.value_table,
        vec![
            (0, "SELECT_HELLO".to_string()),
            (1, "SELECT_WORLD".to_string()),
            (3, "INVALID_SELECTION".to_string()),
        ]
    );

    // Test decoding Message2 (ID 6): signal4 is at start_bit 30, length 4.
    // Raw value 1 -> text "one", value 1.0
    let mut payload1 = [0u8; 7];
    payload1[3] = 0b0100_0000;
    let decoded1 = db.decode(6, &payload1);
    let s4_1 = decoded1.iter().find(|s| s.name == "signal4").expect("signal4");
    assert_eq!(s4_1.value, 1.0);
    assert_eq!(s4_1.text, Some("one"));

    // Raw value 2 -> text "two", value 2.0
    let mut payload2 = [0u8; 7];
    payload2[3] = 0b1000_0000;
    let decoded2 = db.decode(6, &payload2);
    let s4_2 = decoded2.iter().find(|s| s.name == "signal4").expect("signal4");
    assert_eq!(s4_2.value, 2.0);
    assert_eq!(s4_2.text, Some("two"));

    // Raw value 0 (unmapped in table) -> text None, value 0.0
    let payload0 = [0u8; 7];
    let decoded0 = db.decode(6, &payload0);
    let s4_0 = decoded0.iter().find(|s| s.name == "signal4").expect("signal4");
    assert_eq!(s4_0.value, 0.0);
    assert_eq!(s4_0.text, None);

    // Raw value 3 (unmapped in table) -> text None, value 3.0
    let mut payload3 = [0u8; 7];
    payload3[3] = 0b1100_0000;
    let decoded3 = db.decode(6, &payload3);
    let s4_3 = decoded3.iter().find(|s| s.name == "signal4").expect("signal4");
    assert_eq!(s4_3.value, 3.0);
    assert_eq!(s4_3.text, None);
}

/// A `SCALE_LINEAR_AND_TEXTTABLE` compu method retains its rational coefficients
/// (factor, offset, unit) while also collecting the text table scales.
#[test]
fn scale_linear_and_texttable_preserves_scaling_and_value_table() {
    let Some(db) = database() else { return };

    let sig6 = signal(&db, 5, "signal6");
    assert_eq!(sig6.factor, 0.1);
    assert_eq!(sig6.offset, 0.0);
    assert_eq!(sig6.unit, "wp");
    assert_eq!(sig6.value_table, vec![(0, "zero".to_string())]);

    // signal6 is 1 bit at bit 32 (bit 0 of byte 4).
    // Raw value 0: matches text table "zero", value is 0.0 * 0.1 + 0.0 = 0.0
    let mut payload0 = [0u8; 9];
    payload0[4] = 0b0000_0000;
    let decoded0 = db.decode(5, &payload0);
    let s6_0 = decoded0.iter().find(|s| s.name == "signal6").expect("signal6");
    assert_eq!(s6_0.value, 0.0);
    assert_eq!(s6_0.unit, "wp");
    assert_eq!(s6_0.text, Some("zero"));

    // Raw value 1: not in text table (only 0 is "zero"), value is 1.0 * 0.1 + 0.0 = 0.1
    let mut payload1 = [0u8; 9];
    payload1[4] = 0b0000_0001;
    let decoded1 = db.decode(5, &payload1);
    let s6_1 = decoded1.iter().find(|s| s.name == "signal6").expect("signal6");
    assert_eq!(s6_1.value, 0.1);
    assert_eq!(s6_1.unit, "wp");
    assert_eq!(s6_1.text, None);
}

/// A small synthetic ARXML fixture with both TEXTTABLE and SCALE_LINEAR_AND_TEXTTABLE.
#[test]
fn synthetic_arxml_texttable_fixture_decodes() {
    let arxml_content = r#"<?xml version="1.0" encoding="utf-8"?>
<AUTOSAR xmlns="http://autosar.org/schema/r4.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://autosar.org/schema/r4.0 AUTOSAR_4-2-2.xsd">
  <AR-PACKAGES>
    <AR-PACKAGE>
      <SHORT-NAME>TestPackage</SHORT-NAME>
      <ELEMENTS>
        <COMPU-METHOD>
          <SHORT-NAME>StateCompu</SHORT-NAME>
          <CATEGORY>TEXTTABLE</CATEGORY>
          <COMPU-INTERNAL-TO-PHYS>
            <COMPU-SCALES>
              <COMPU-SCALE>
                <LOWER-LIMIT>0</LOWER-LIMIT>
                <UPPER-LIMIT>0</UPPER-LIMIT>
                <COMPU-CONST>
                  <VT>Off</VT>
                </COMPU-CONST>
              </COMPU-SCALE>
              <COMPU-SCALE>
                <LOWER-LIMIT>1</LOWER-LIMIT>
                <UPPER-LIMIT>1</UPPER-LIMIT>
                <COMPU-CONST>
                  <VT>On</VT>
                </COMPU-CONST>
              </COMPU-SCALE>
              <COMPU-SCALE>
                <LOWER-LIMIT>2</LOWER-LIMIT>
                <UPPER-LIMIT>2</UPPER-LIMIT>
                <COMPU-CONST>
                  <VT>Error</VT>
                </COMPU-CONST>
              </COMPU-SCALE>
            </COMPU-SCALES>
          </COMPU-INTERNAL-TO-PHYS>
        </COMPU-METHOD>
        <COMPU-METHOD>
          <SHORT-NAME>LinearTextCompu</SHORT-NAME>
          <CATEGORY>SCALE_LINEAR_AND_TEXTTABLE</CATEGORY>
          <UNIT-REF DEST="UNIT">/TestPackage/KmPerHour</UNIT-REF>
          <COMPU-INTERNAL-TO-PHYS>
            <COMPU-SCALES>
              <COMPU-SCALE>
                <LOWER-LIMIT>255</LOWER-LIMIT>
                <UPPER-LIMIT>255</UPPER-LIMIT>
                <COMPU-CONST>
                  <VT>Invalid</VT>
                </COMPU-CONST>
              </COMPU-SCALE>
              <COMPU-SCALE>
                <LOWER-LIMIT>0</LOWER-LIMIT>
                <UPPER-LIMIT>250</UPPER-LIMIT>
                <COMPU-RATIONAL-COEFFS>
                  <COMPU-NUMERATOR>
                    <V>0</V>
                    <V>0.5</V>
                  </COMPU-NUMERATOR>
                  <COMPU-DENOMINATOR>
                    <V>1</V>
                  </COMPU-DENOMINATOR>
                </COMPU-RATIONAL-COEFFS>
              </COMPU-SCALE>
            </COMPU-SCALES>
          </COMPU-INTERNAL-TO-PHYS>
        </COMPU-METHOD>
        <UNIT>
          <SHORT-NAME>KmPerHour</SHORT-NAME>
          <DISPLAY-NAME>km/h</DISPLAY-NAME>
        </UNIT>
        <SYSTEM-SIGNAL>
          <SHORT-NAME>StateSysSig</SHORT-NAME>
          <PHYSICAL-PROPS>
            <SW-DATA-DEF-PROPS-VARIANTS>
              <SW-DATA-DEF-PROPS-CONDITIONAL>
                <COMPU-METHOD-REF DEST="COMPU-METHOD">/TestPackage/StateCompu</COMPU-METHOD-REF>
              </SW-DATA-DEF-PROPS-CONDITIONAL>
            </SW-DATA-DEF-PROPS-VARIANTS>
          </PHYSICAL-PROPS>
        </SYSTEM-SIGNAL>
        <SYSTEM-SIGNAL>
          <SHORT-NAME>SpeedSysSig</SHORT-NAME>
          <PHYSICAL-PROPS>
            <SW-DATA-DEF-PROPS-VARIANTS>
              <SW-DATA-DEF-PROPS-CONDITIONAL>
                <COMPU-METHOD-REF DEST="COMPU-METHOD">/TestPackage/LinearTextCompu</COMPU-METHOD-REF>
              </SW-DATA-DEF-PROPS-CONDITIONAL>
            </SW-DATA-DEF-PROPS-VARIANTS>
          </PHYSICAL-PROPS>
        </SYSTEM-SIGNAL>
        <I-SIGNAL>
          <SHORT-NAME>StateSig</SHORT-NAME>
          <LENGTH>8</LENGTH>
          <SYSTEM-SIGNAL-REF DEST="SYSTEM-SIGNAL">/TestPackage/StateSysSig</SYSTEM-SIGNAL-REF>
        </I-SIGNAL>
        <I-SIGNAL>
          <SHORT-NAME>SpeedSig</SHORT-NAME>
          <LENGTH>8</LENGTH>
          <SYSTEM-SIGNAL-REF DEST="SYSTEM-SIGNAL">/TestPackage/SpeedSysSig</SYSTEM-SIGNAL-REF>
        </I-SIGNAL>
        <I-SIGNAL-I-PDU>
          <SHORT-NAME>TestPDU</SHORT-NAME>
          <LENGTH>2</LENGTH>
          <I-SIGNAL-TO-PDU-MAPPINGS>
            <I-SIGNAL-TO-I-PDU-MAPPING>
              <SHORT-NAME>StateMapping</SHORT-NAME>
              <I-SIGNAL-REF DEST="I-SIGNAL">/TestPackage/StateSig</I-SIGNAL-REF>
              <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
              <START-POSITION>0</START-POSITION>
            </I-SIGNAL-TO-I-PDU-MAPPING>
            <I-SIGNAL-TO-I-PDU-MAPPING>
              <SHORT-NAME>SpeedMapping</SHORT-NAME>
              <I-SIGNAL-REF DEST="I-SIGNAL">/TestPackage/SpeedSig</I-SIGNAL-REF>
              <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
              <START-POSITION>8</START-POSITION>
            </I-SIGNAL-TO-I-PDU-MAPPING>
          </I-SIGNAL-TO-PDU-MAPPINGS>
        </I-SIGNAL-I-PDU>
        <CAN-FRAME>
          <SHORT-NAME>TestFrame</SHORT-NAME>
          <FRAME-LENGTH>2</FRAME-LENGTH>
          <PDU-TO-FRAME-MAPPINGS>
            <PDU-TO-FRAME-MAPPING>
              <SHORT-NAME>PduMapping</SHORT-NAME>
              <PDU-REF DEST="I-SIGNAL-I-PDU">/TestPackage/TestPDU</PDU-REF>
            </PDU-TO-FRAME-MAPPING>
          </PDU-TO-FRAME-MAPPINGS>
        </CAN-FRAME>
        <CAN-CLUSTER>
          <SHORT-NAME>CanCluster</SHORT-NAME>
          <CAN-CLUSTER-VARIANTS>
            <CAN-CLUSTER-CONDITIONAL>
              <PHYSICAL-CHANNELS>
                <CAN-PHYSICAL-CHANNEL>
                  <SHORT-NAME>CanChannel</SHORT-NAME>
                  <FRAME-TRIGGERINGS>
                    <CAN-FRAME-TRIGGERING>
                      <SHORT-NAME>FrameTriggering</SHORT-NAME>
                      <CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE>
                      <FRAME-REF DEST="CAN-FRAME">/TestPackage/TestFrame</FRAME-REF>
                      <IDENTIFIER>42</IDENTIFIER>
                    </CAN-FRAME-TRIGGERING>
                  </FRAME-TRIGGERINGS>
                </CAN-PHYSICAL-CHANNEL>
              </PHYSICAL-CHANNELS>
            </CAN-CLUSTER-CONDITIONAL>
          </CAN-CLUSTER-VARIANTS>
        </CAN-CLUSTER>
      </ELEMENTS>
    </AR-PACKAGE>
  </AR-PACKAGES>
</AUTOSAR>
"#;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("fixture.arxml");
    std::fs::write(&path, arxml_content).expect("write fixture");

    let db = CanDatabase::from_arxml_path(&path).expect("load synthetic ARXML");
    let msg = db.message(42).expect("message 42");
    assert_eq!(msg.name, "TestFrame");
    assert_eq!(msg.length, 2);

    let state = msg.signals.iter().find(|s| s.name == "StateSig").expect("StateSig");
    assert_eq!(
        state.value_table,
        vec![
            (0, "Off".to_string()),
            (1, "On".to_string()),
            (2, "Error".to_string()),
        ]
    );

    let speed = msg.signals.iter().find(|s| s.name == "SpeedSig").expect("SpeedSig");
    assert_eq!(speed.factor, 0.5);
    assert_eq!(speed.offset, 0.0);
    assert_eq!(speed.unit, "km/h");
    assert_eq!(speed.value_table, vec![(255, "Invalid".to_string())]);

    // Decode sample payload: byte 0 = 1 (State: On), byte 1 = 100 (Speed: 50.0 km/h)
    let payload = [1u8, 100u8];
    let decoded = db.decode(42, &payload);

    let dec_state = decoded.iter().find(|s| s.name == "StateSig").expect("StateSig decoded");
    assert_eq!(dec_state.value, 1.0);
    assert_eq!(dec_state.text, Some("On"));

    let dec_speed = decoded.iter().find(|s| s.name == "SpeedSig").expect("SpeedSig decoded");
    assert_eq!(dec_speed.value, 50.0);
    assert_eq!(dec_speed.unit, "km/h");
    assert_eq!(dec_speed.text, None);

    // Decode unmapped state (e.g. 5) and error speed (255)
    let payload2 = [5u8, 255u8];
    let decoded2 = db.decode(42, &payload2);

    let dec_state2 = decoded2.iter().find(|s| s.name == "StateSig").expect("StateSig decoded");
    assert_eq!(dec_state2.value, 5.0);
    assert_eq!(dec_state2.text, None);

    let dec_speed2 = decoded2.iter().find(|s| s.name == "SpeedSig").expect("SpeedSig decoded");
    assert_eq!(dec_speed2.value, 127.5);
    assert_eq!(dec_speed2.unit, "km/h");
    assert_eq!(dec_speed2.text, Some("Invalid"));
}

