//! MDF 3.x sample decoding, checked against asammdf reading the same bytes.
//!
//! Two kinds of fixture appear here, and neither takes its expected values
//! from this crate:
//!
//! * files asammdf **writes**, read back by asammdf and by falcon;
//! * files this test **builds byte by byte** from the format's block layouts,
//!   read by asammdf and by falcon. asammdf can only write sorted,
//!   little-endian v3, so the record-identifier and Motorola cases have no
//!   other way to exist. For those, the values written are also asserted
//!   directly against the constants that went in, so a bug in the builder
//!   cannot make both readers agree on the wrong answer quietly.
//!
//! Three silent data-corruption defects in this repository survived their
//! tests because those tests used the implementation's own inverse as their
//! oracle. Nothing here does that.

#![cfg(feature = "mdf3")]

use std::path::{Path, PathBuf};

use falcon_mdf::mdf3::Mdf3File;
use falcon_mdf::SignalValues;

mod common;
use common::{
    asammdf_raw_samples, assert_same_samples, build_v3, poke_be, poke_le, python_json,
    write_synthetic, Ch, Grp,
};

// ---------------------------------------------------------------------------
// Files asammdf wrote
// ---------------------------------------------------------------------------

/// Writes a v3 file with asammdf covering the types it is able to emit.
fn generate_typed_file(version: &str, dir: &Path) -> PathBuf {
    let path = dir.join(format!("typed_{}.mdf", version.replace('.', "_")));
    python_json(&format!(
        r#"
import json
import numpy as np
from asammdf import MDF, Signal

t = np.arange(0.0, 1.0, 0.1)
sigs = [
    Signal(samples=(t * 3.0).astype(np.float64), timestamps=t, name="Speed", unit="km/h"),
    Signal(samples=(t * 1.5).astype(np.float32), timestamps=t, name="Volt", unit="V"),
    Signal(samples=(t * 100 - 50).astype(np.int16), timestamps=t, name="Rpm", unit="1/min"),
    Signal(samples=(t * 7 - 3).astype(np.int32), timestamps=t, name="Torque", unit="Nm"),
    Signal(samples=(t * 20).astype(np.uint8), timestamps=t, name="Gear", unit=""),
    Signal(samples=(t * 1e6).astype(np.uint32), timestamps=t, name="Count", unit=""),
    Signal(
        samples=np.array([("state_%02d" % i).encode() for i in range(10)], dtype="S12"),
        timestamps=t,
        name="Label",
        unit="",
    ),
]
m = MDF(version="{version}")
m.append(sigs)
m.save(r"{path}", overwrite=True)
m.close()
print(json.dumps("written"))
"#,
        version = version,
        path = path.display()
    ));
    path
}

#[test]
fn samples_asammdf_wrote_read_back_sample_for_sample() {
    let dir = tempfile::tempdir().expect("a temp dir");

    for version in ["3.30", "3.20", "2.14"] {
        let path = generate_typed_file(version, dir.path());
        let expected = asammdf_raw_samples(&path);
        let expected = expected.as_array().expect("a channel list");

        let file = Mdf3File::open(&path).expect("falcon should open it");

        let mut seen = 0;
        for (g, dg) in file.data_groups().iter().enumerate() {
            for (c, cg) in dg.channel_groups.iter().enumerate() {
                for (i, channel) in cg.channels.iter().enumerate() {
                    let want = &expected[seen];
                    assert_eq!(
                        want["name"].as_str().unwrap(),
                        channel.name,
                        "{version}: falcon and asammdf should walk the channels in the same order"
                    );
                    let got = file
                        .channel_values(g, c, i)
                        .unwrap_or_else(|e| panic!("{version}: reading {} failed: {e}", channel.name));
                    assert_eq!(
                        got.len(),
                        10,
                        "{version}: {} should have the ten cycles asammdf wrote",
                        channel.name
                    );
                    assert_same_samples(&format!("{version}/{}", channel.name), &got, want);
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, expected.len(), "{version}: every channel should be read");
    }
}

#[test]
fn an_integer_channel_keeps_its_own_width_rather_than_going_through_f64() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = generate_typed_file("3.30", dir.path());
    let file = Mdf3File::open(&path).expect("falcon should open it");

    // The widths asammdf wrote for these signals, checked separately in
    // `samples_asammdf_wrote_read_back_sample_for_sample`; here it is the Rust
    // type that matters, not the values.
    for (name, kind) in [
        ("Speed", "f64"),
        ("Volt", "f32"),
        ("Rpm", "i16"),
        ("Torque", "i32"),
        ("Gear", "u8"),
        ("Count", "u32"),
        ("Label", "str"),
    ] {
        let values = file
            .values_by_name(name)
            .unwrap_or_else(|e| panic!("reading {name} failed: {e}"));
        assert_eq!(
            values.kind().name(),
            kind,
            "{name} should decode to its own type, not to f64"
        );
    }
}

#[test]
fn the_time_channel_is_found_and_decodes_to_its_stored_type() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = generate_typed_file("3.30", dir.path());
    let expected = asammdf_raw_samples(&path);
    let file = Mdf3File::open(&path).expect("falcon should open it");

    let master = file.master_channel(0, 0).expect("the group has a time channel");
    assert!(master.is_time(), "the master should carry the time flag");

    let index = file.data_groups()[0].channel_groups[0]
        .channels
        .iter()
        .position(|c| c.name == master.name)
        .unwrap();
    let got = file.channel_values(0, 0, index).expect("reading the time channel");
    let want = expected
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"].as_str() == Some(master.name.as_str()))
        .expect("asammdf should report the same time channel");
    assert_same_samples("time", &got, want);
}

// ---------------------------------------------------------------------------
// Cases asammdf cannot write, built here and read by both
// ---------------------------------------------------------------------------

/// The values a [`bit_packed_group`] record stream carries, as they went in.
struct BitPacked {
    le_u: Vec<i64>,
    le_i: Vec<i64>,
    be_u: Vec<i64>,
    be_i: Vec<i64>,
    be_f: Vec<f64>,
    raw: Vec<[u8; 3]>,
}

/// Builds a group whose channels are packed at bit boundaries, in both byte
/// orders, and returns it alongside the values that went in.
fn bit_packed_group(record_id: u16) -> (Grp, BitPacked) {
    // Two 12-bit fields per byte order, one unsigned and one signed, none of
    // them a whole number of bytes; then a Motorola double and a Motorola
    // float.
    //
    //   bits   0..64  time, IEEE double, Intel   bytes  0..8
    //   bits  64..76  u12, Intel                 bytes  8..10, low 12 bits
    //   bits  76..88  i12, Intel                 bytes  9..11, high 12 bits
    //   bits  88..100 u12, Motorola              bytes 11..13, low 12 bits
    //   bits 108..120 i12, Motorola              bytes 13..15, high 12 bits
    //   bits 120..184 f64, Motorola              bytes 15..23
    //   bits 184..216 f32, Motorola              bytes 23..27
    //   bits 216..240 byte array                 bytes 27..30
    //
    // The two byte orders do not tile a record the same way. A little-endian
    // field of 12 bits at bit 64 occupies the low 12 bits of the two bytes it
    // spans, so the next field starts at bit 76 and they pack. A big-endian
    // field of 12 bits at bit 88 also takes the low 12 bits of *its* window —
    // which are the second byte and the low nibble of the first — so the next
    // one cannot start at bit 100 without overlapping it. Each Motorola field
    // is therefore given its own bytes, with the spare nibble left unused.
    let record_size = 30usize;
    let v = BitPacked {
        le_u: vec![0, 1, 2047, 2048, 4095],
        le_i: vec![0, 1, -1, 2047, -2048],
        be_u: vec![4095, 2048, 2047, 1, 0],
        be_i: vec![-2048, 2047, -1, 1, 0],
        be_f: vec![0.0, -1.5, 1e10, -2.5e-7, 12345.678],
        raw: vec![
            [0x00, 0x00, 0x00],
            [0xDE, 0xAD, 0xBE],
            [0xFF, 0xFF, 0xFF],
            [0x01, 0x80, 0x7F],
            [0x10, 0x20, 0x30],
        ],
    };

    let mut records = Vec::new();
    for i in 0..5 {
        let mut r = vec![0u8; record_size];
        r[0..8].copy_from_slice(&(i as f64 * 0.1).to_le_bytes());
        poke_le(&mut r, 64, 12, v.le_u[i] as u64);
        poke_le(&mut r, 76, 12, v.le_i[i] as u64 & 0xFFF);
        poke_be(&mut r, 88, 12, v.be_u[i] as u64);
        poke_be(&mut r, 108, 12, v.be_i[i] as u64 & 0xFFF);
        r[15..23].copy_from_slice(&v.be_f[i].to_be_bytes());
        r[23..27].copy_from_slice(&(v.be_f[i] as f32).to_be_bytes());
        r[27..30].copy_from_slice(&v.raw[i]);
        records.push(r);
    }

    let grp = Grp {
        record_id,
        record_size: record_size as u16,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("LeU12", 0, 64, 12, 13),
            Ch::new("LeI12", 0, 76, 12, 14),
            Ch::new("BeU12", 0, 88, 12, 9),
            Ch::new("BeI12", 0, 108, 12, 10),
            Ch::new("BeF64", 0, 120, 64, 12),
            Ch::new("BeF32", 0, 184, 32, 11),
            Ch::new("Raw", 0, 216, 24, 8),
        ],
        records,
    };
    (grp, v)
}

/// Reads a channel and returns its samples widened to i64, failing if it did
/// not decode to an integer.
fn ints(file: &Mdf3File, name: &str) -> Vec<i64> {
    match file.values_by_name(name).unwrap_or_else(|e| panic!("reading {name}: {e}")) {
        SignalValues::U8(v) => v.iter().map(|&x| x as i64).collect(),
        SignalValues::U16(v) => v.iter().map(|&x| x as i64).collect(),
        SignalValues::U32(v) => v.iter().map(|&x| x as i64).collect(),
        SignalValues::I8(v) => v.iter().map(|&x| x as i64).collect(),
        SignalValues::I16(v) => v.iter().map(|&x| x as i64).collect(),
        SignalValues::I32(v) => v.iter().map(|&x| x as i64).collect(),
        other => panic!("{name} should decode to an integer, got {:?}", other.kind()),
    }
}

#[test]
fn non_byte_aligned_fields_decode_in_both_byte_orders() {
    // Big-endian v3 cannot be produced by asammdf's writer, so this file is
    // built here from the block layouts. asammdf still reads it, and is still
    // the oracle for the values.
    let dir = tempfile::tempdir().expect("a temp dir");
    let (grp, v) = bit_packed_group(0);
    let bytes = build_v3(&[grp], 0, &[0, 0, 0, 0, 0]);
    let path = write_synthetic(dir.path(), "bitpacked.mdf", &bytes);

    let file = Mdf3File::open(&path).expect("falcon should open the synthetic file");

    // First: against the constants that went in, so a builder bug cannot hide
    // behind two readers agreeing.
    assert_eq!(ints(&file, "LeU12"), v.le_u, "little-endian unsigned 12-bit fields");
    assert_eq!(ints(&file, "LeI12"), v.le_i, "little-endian signed 12-bit fields");
    assert_eq!(ints(&file, "BeU12"), v.be_u, "big-endian unsigned 12-bit fields");
    assert_eq!(ints(&file, "BeI12"), v.be_i, "big-endian signed 12-bit fields");
    match file.values_by_name("BeF64").unwrap() {
        SignalValues::F64(g) => assert_eq!(g, v.be_f, "big-endian doubles"),
        other => panic!("BeF64 should decode to f64, got {:?}", other.kind()),
    }
    match file.values_by_name("BeF32").unwrap() {
        SignalValues::F32(g) => {
            let want: Vec<f32> = v.be_f.iter().map(|&x| x as f32).collect();
            assert_eq!(g, want, "big-endian floats");
        }
        other => panic!("BeF32 should decode to f32, got {:?}", other.kind()),
    }
    match file.values_by_name("Raw").unwrap() {
        SignalValues::Bytes { data, width } => {
            assert_eq!(width, 3, "a 24-bit byte array is three bytes per sample");
            assert_eq!(data, v.raw.concat(), "byte-array samples");
        }
        other => panic!("Raw should decode to bytes, got {:?}", other.kind()),
    }

    // Then: against asammdf reading the same bytes.
    let expected = asammdf_raw_samples(&path);
    for want in expected.as_array().unwrap() {
        let name = want["name"].as_str().unwrap();
        let got = file
            .values_by_name(name)
            .unwrap_or_else(|e| panic!("reading {name}: {e}"));
        assert_same_samples(name, &got, want);
    }
}

/// Builds two channel groups sharing one data group, with their records
/// interleaved. asammdf's writer always emits sorted v3, so the only way to
/// exercise record identifiers is to lay the bytes out here.
fn two_group_file(record_id_count: u16) -> (Vec<u8>, Vec<u32>, Vec<i32>) {
    let a: Vec<u32> = vec![7, 11, 13, 17, 19, 23];
    let b: Vec<i32> = vec![-1, -2, -3, -4];

    let ga = Grp {
        record_id: 1,
        record_size: 12,
        channels: vec![
            Ch::new("time_a", 1, 0, 64, 3),
            Ch::new("A", 0, 64, 32, 13),
        ],
        records: a
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let mut r = vec![0u8; 12];
                r[0..8].copy_from_slice(&(i as f64).to_le_bytes());
                r[8..12].copy_from_slice(&v.to_le_bytes());
                r
            })
            .collect(),
    };
    let gb = Grp {
        record_id: 2,
        record_size: 10,
        channels: vec![
            Ch::new("time_b", 1, 0, 64, 3),
            Ch::new("B", 0, 64, 16, 14),
        ],
        records: b
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let mut r = vec![0u8; 10];
                r[0..8].copy_from_slice(&(i as f64 * 2.0).to_le_bytes());
                r[8..10].copy_from_slice(&(v as i16).to_le_bytes());
                r
            })
            .collect(),
    };

    // Deliberately uneven: the two groups have different lengths and different
    // record sizes, so a reader that assumed a single stride would go wrong on
    // the third record.
    let order = vec![1u16, 2, 1, 1, 2, 2, 1, 1, 2, 1];
    (build_v3(&[ga, gb], record_id_count, &order), a, b)
}

#[test]
fn two_channel_groups_in_one_data_group_are_told_apart_by_record_id() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (bytes, a, b) = two_group_file(1);
    let path = write_synthetic(dir.path(), "unsorted.mdf", &bytes);

    let file = Mdf3File::open(&path).expect("falcon should open the synthetic file");
    assert_eq!(
        file.data_groups()[0].record_id_count,
        1,
        "the data group should report one record identifier per record"
    );

    match file.values_by_name("A").unwrap() {
        SignalValues::U32(v) => assert_eq!(v, a, "group 1's samples"),
        other => panic!("A should decode to u32, got {:?}", other.kind()),
    }
    match file.values_by_name("B").unwrap() {
        SignalValues::I16(v) => {
            let want: Vec<i16> = b.iter().map(|&x| x as i16).collect();
            assert_eq!(v, want, "group 2's samples");
        }
        other => panic!("B should decode to i16, got {:?}", other.kind()),
    }

    let expected = asammdf_raw_samples(&path);
    for want in expected.as_array().unwrap() {
        let name = want["name"].as_str().unwrap();
        let got = file.values_by_name(name).unwrap_or_else(|e| panic!("reading {name}: {e}"));
        assert_same_samples(name, &got, want);
    }
}

#[test]
fn a_record_identifier_repeated_after_the_record_is_honoured() {
    // The variant where the identifier appears both before and after each
    // record. Reading the field as a byte width instead of a count of
    // identifiers puts every record after the first one byte out, which decodes
    // to plausible wrong numbers rather than failing.
    let dir = tempfile::tempdir().expect("a temp dir");
    let (bytes, a, b) = two_group_file(2);
    let path = write_synthetic(dir.path(), "trailing_id.mdf", &bytes);

    let file = Mdf3File::open(&path).expect("falcon should open the synthetic file");
    assert_eq!(file.data_groups()[0].record_id_count, 2);

    match file.values_by_name("A").unwrap() {
        SignalValues::U32(v) => assert_eq!(v, a),
        other => panic!("A should decode to u32, got {:?}", other.kind()),
    }
    match file.values_by_name("B").unwrap() {
        SignalValues::I16(v) => {
            let want: Vec<i16> = b.iter().map(|&x| x as i16).collect();
            assert_eq!(v, want);
        }
        other => panic!("B should decode to i16, got {:?}", other.kind()),
    }

    let expected = asammdf_raw_samples(&path);
    for want in expected.as_array().unwrap() {
        let name = want["name"].as_str().unwrap();
        let got = file.values_by_name(name).unwrap_or_else(|e| panic!("reading {name}: {e}"));
        assert_same_samples(name, &got, want);
    }
}

// ---------------------------------------------------------------------------
// What must fail rather than return a number
// ---------------------------------------------------------------------------

/// Builds a one-group file whose single data channel has the given placement,
/// with `record_size` bytes per record.
fn placement_file(start_offset: u16, bit_count: u16, record_size: u16, data_type: u16) -> Vec<u8> {
    let grp = Grp {
        record_id: 0,
        record_size,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("X", 0, start_offset, bit_count, data_type),
        ],
        records: (0..3).map(|_| vec![0u8; record_size as usize]).collect(),
    };
    build_v3(&[grp], 0, &[0, 0, 0])
}

#[test]
fn a_channel_that_does_not_fit_its_record_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");

    // A record of 12 bytes has 96 bits. Each of these asks for bits past that.
    for (start, bits, size) in [(64u16, 64u16, 12u16), (88, 16, 12), (95, 2, 12), (768, 8, 12)] {
        let bytes = placement_file(start, bits, size, 13);
        let path = write_synthetic(
            dir.path(),
            &format!("overrun_{start}_{bits}.mdf"),
            &bytes,
        );
        let file = Mdf3File::open(&path).expect("the structure itself is well formed");
        let err = match file.values_by_name("X") {
            Ok(v) => panic!(
                "a channel at bits {start}..{} of a {size}-byte record must be refused, \
                 got {} samples",
                start as usize + bits as usize,
                v.len()
            ),
            Err(e) => e,
        };
        assert!(
            matches!(err, falcon_mdf::Mf4Error::InvalidDataBlock { .. }),
            "the refusal should name the record layout, got: {err}"
        );
        assert!(
            err.to_string().contains("record"),
            "the refusal should say what did not fit, got: {err}"
        );
    }
}

#[test]
fn a_channel_that_exactly_fills_its_record_is_read() {
    // The boundary the previous test sits just past: 96 bits is the last bit a
    // 12-byte record has, and rejecting it would be an off-by-one the other
    // way.
    let dir = tempfile::tempdir().expect("a temp dir");
    let bytes = placement_file(64, 32, 12, 13);
    let path = write_synthetic(dir.path(), "exact_fit.mdf", &bytes);
    let file = Mdf3File::open(&path).expect("falcon should open it");
    assert_eq!(
        file.values_by_name("X").expect("a channel ending on the last bit fits").len(),
        3
    );
}

#[test]
fn a_data_block_shorter_than_the_declared_cycles_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let bytes = placement_file(64, 32, 12, 13);

    // Cut the file so the last record is incomplete. The cycle count still says
    // three, and reading two and calling it three would drop a sample silently.
    let path = write_synthetic(dir.path(), "short_data.mdf", &bytes[..bytes.len() - 5]);
    let file = Mdf3File::open(&path).expect("the structure is intact; only the records are cut");
    let err = match file.values_by_name("X") {
        Ok(v) => panic!("a truncated record stream must be refused, got {} samples", v.len()),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::TruncatedFile { .. }),
        "the refusal should say the file is short, got: {err}"
    );
}

#[test]
fn a_record_carrying_an_unclaimed_identifier_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (mut bytes, _, _) = two_group_file(1);

    // Find the last record's leading identifier and change it to one no channel
    // group claims. Its length is then unknown, so every byte after it would be
    // read at the wrong offset.
    let last = bytes.len() - 13;
    assert_eq!(bytes[last], 1, "the stream should end with a group 1 record");
    bytes[last] = 9;
    let path = write_synthetic(dir.path(), "stray_id.mdf", &bytes);

    let file = Mdf3File::open(&path).expect("the structure is well formed");
    let err = match file.values_by_name("A") {
        Ok(v) => panic!("an unknown record identifier must be refused, got {} samples", v.len()),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidDataBlock { .. }),
        "the refusal should name the identifier, got: {err}"
    );
    assert!(err.to_string().contains('9'), "the refusal should say which one: {err}");
}

#[test]
fn a_trailing_identifier_that_does_not_match_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (mut bytes, _, _) = two_group_file(2);

    // The last record is group 1's: identifier, twelve bytes, identifier. Break
    // the trailing copy — the guard that catches a stream read one byte out.
    let last = bytes.len() - 1;
    assert_eq!(bytes[last], 1);
    bytes[last] = 2;
    let path = write_synthetic(dir.path(), "bad_trailing_id.mdf", &bytes);

    let file = Mdf3File::open(&path).expect("the structure is well formed");
    let err = match file.values_by_name("A") {
        Ok(v) => panic!("a mismatched trailing identifier must be refused, got {} samples", v.len()),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidDataBlock { .. }),
        "the refusal should name the mismatch, got: {err}"
    );
}

#[test]
fn a_data_type_this_build_cannot_decode_is_refused_rather_than_guessed() {
    let dir = tempfile::tempdir().expect("a temp dir");

    for code in [99u16, 100] {
        let bytes = placement_file(64, 32, 12, code);
        let path = write_synthetic(dir.path(), &format!("unsupported_{code}.mdf"), &bytes);
        let file = Mdf3File::open(&path).expect("falcon should open it");
        let err = match file.values_by_name("X") {
            Ok(_) => panic!("data type {code} must be refused"),
            Err(e) => e,
        };
        assert!(
            matches!(err, falcon_mdf::Mf4Error::Unsupported { .. }),
            "the refusal should name the type, got: {err}"
        );
    }
}

#[test]
fn synthetic_file_with_vax_float_channels_decodes_correctly() {
    let dir = tempfile::tempdir().expect("a temp dir");

    // Test VAX F (code 4): 32-bit float
    let grp_f = Grp {
        record_id: 0,
        record_size: 12,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("vax_f", 0, 64, 32, 4),
        ],
        records: vec![
            // sample 0: time=0.0, vax_f=1.0 ([0x80, 0x40, 0x00, 0x00])
            [0u8; 8].into_iter().chain([0x80, 0x40, 0x00, 0x00]).collect(),
            // sample 1: time=0.0, vax_f=0.5 ([0x00, 0x40, 0x00, 0x00])
            [0u8; 8].into_iter().chain([0x00, 0x40, 0x00, 0x00]).collect(),
            // sample 2: time=0.0, vax_f=-2.5 ([0x20, 0xC1, 0x00, 0x00])
            [0u8; 8].into_iter().chain([0x20, 0xC1, 0x00, 0x00]).collect(),
        ],
    };
    let bytes_f = build_v3(&[grp_f], 0, &[0, 0, 0]);
    let path_f = write_synthetic(dir.path(), "vax_f.mdf", &bytes_f);
    let file_f = Mdf3File::open(&path_f).expect("file should open");
    let values_f = file_f.values_by_name("vax_f").expect("channel should decode");
    assert_eq!(values_f, falcon_mdf::SignalValues::F64(vec![1.0, 0.5, -2.5]));

    // Test VAX D (code 5): 64-bit float
    let grp_d = Grp {
        record_id: 0,
        record_size: 16,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("vax_d", 0, 64, 64, 5),
        ],
        records: vec![
            [0u8; 8].into_iter().chain([0x80, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).collect(),
            [0u8; 8].into_iter().chain([0x20, 0xC1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).collect(),
        ],
    };
    let bytes_d = build_v3(&[grp_d], 0, &[0, 0]);
    let path_d = write_synthetic(dir.path(), "vax_d.mdf", &bytes_d);
    let file_d = Mdf3File::open(&path_d).expect("file should open");
    let values_d = file_d.values_by_name("vax_d").expect("channel should decode");
    assert_eq!(values_d, falcon_mdf::SignalValues::F64(vec![1.0, -2.5]));

    // Test VAX G (code 6): 64-bit float
    let grp_g = Grp {
        record_id: 0,
        record_size: 16,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("vax_g", 0, 64, 64, 6),
        ],
        records: vec![
            [0u8; 8].into_iter().chain([0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).collect(),
            [0u8; 8].into_iter().chain([0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).collect(),
            [0u8; 8].into_iter().chain([0x24, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).collect(),
        ],
    };
    let bytes_g = build_v3(&[grp_g], 0, &[0, 0, 0]);
    let path_g = write_synthetic(dir.path(), "vax_g.mdf", &bytes_g);
    let file_g = Mdf3File::open(&path_g).expect("file should open");
    let values_g = file_g.values_by_name("vax_g").expect("channel should decode");
    assert_eq!(values_g, falcon_mdf::SignalValues::F64(vec![1.0, 0.5, -2.5]));
}

#[test]
fn a_float_of_a_width_ieee_does_not_define_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let bytes = placement_file(64, 24, 12, 3);
    let path = write_synthetic(dir.path(), "float24.mdf", &bytes);
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let err = match file.values_by_name("X") {
        Ok(_) => panic!("a 24-bit float must be refused, not rounded to 32 bits"),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::Unsupported { .. }),
        "the refusal should name the width, got: {err}"
    );
}
