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
