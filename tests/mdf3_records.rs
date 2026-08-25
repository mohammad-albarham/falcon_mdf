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
use std::process::Command;

use falcon_mdf::mdf3::Mdf3File;
use falcon_mdf::SignalValues;

// ---------------------------------------------------------------------------
// asammdf, as the oracle
// ---------------------------------------------------------------------------

/// Locates the virtualenv python that has asammdf installed.
fn venv_python() -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .expect("no .venv/bin/python with asammdf found; these tests need it for their oracle")
}

/// Runs a python snippet and parses its last line as JSON.
fn python_json(script: &str) -> serde_json::Value {
    let out = Command::new(venv_python())
        .arg("-c")
        .arg(script)
        .output()
        .expect("running python should succeed");
    assert!(
        out.status.success(),
        "the asammdf oracle failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .last()
        .unwrap_or_else(|| panic!("the oracle printed nothing; stderr: {}", String::from_utf8_lossy(&out.stderr)));
    serde_json::from_str(line).unwrap_or_else(|e| panic!("the oracle's output should be JSON: {e}\n{line}"))
}

/// Asks asammdf what raw samples a v3 file holds, channel by channel.
///
/// `raw=True` keeps conversions out of it, which is what this task decodes.
fn asammdf_raw_samples(path: &Path) -> serde_json::Value {
    python_json(&format!(
        r#"
import json
import struct
import numpy as np
from asammdf import MDF

m = MDF(r"{path}")
out = []
for gi, g in enumerate(m.groups):
    for ci, ch in enumerate(g.channels):
        vals = m.get(group=gi, index=ci, raw=True, samples_only=True)[0]
        vals = np.asarray(vals)
        kind = vals.dtype.kind
        if kind == "S":
            values = [v.split(b"\x00")[0].decode("latin-1") for v in vals.tolist()]
        elif kind in "iub":
            values = [int(v) for v in vals.ravel().tolist()]
            if vals.ndim > 1:
                width = vals.shape[1]
                values = [values[i:i + width] for i in range(0, len(values), width)]
        elif kind == "f":
            # As the double's bit pattern, not as decimal text. A float written
            # out in decimal and read back through a JSON parser can shift by
            # one unit in the last place, which would make this oracle disagree
            # with a correct reader over a value both of them got right.
            values = [
                int.from_bytes(struct.pack("<d", float(v)), "little") for v in vals.tolist()
            ]
        else:
            values = None
        out.append({{"name": ch.name, "kind": kind, "ndim": int(vals.ndim), "values": values}})
print(json.dumps(out))
m.close()
"#,
        path = path.display()
    ))
}

/// Compares falcon's decoded samples with what asammdf reported for the same
/// channel, sample for sample.
fn assert_same_samples(ctx: &str, got: &SignalValues, want: &serde_json::Value) {
    let expected = want["values"]
        .as_array()
        .unwrap_or_else(|| panic!("{ctx}: asammdf reported a kind this test cannot compare: {want}"));

    let unsigned = |v: &[u64]| {
        let e: Vec<u64> = expected.iter().map(|x| x.as_u64().unwrap()).collect();
        assert_eq!(v, e.as_slice(), "{ctx}: samples should equal asammdf's");
    };
    let signed = |v: &[i64]| {
        let e: Vec<i64> = expected.iter().map(|x| x.as_i64().unwrap()).collect();
        assert_eq!(v, e.as_slice(), "{ctx}: samples should equal asammdf's");
    };
    // Floats travel as the double's bit pattern; see the oracle script.
    let float = |v: &[f64]| {
        let g: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
        let e: Vec<u64> = expected.iter().map(|x| x.as_u64().unwrap()).collect();
        assert_eq!(
            g,
            e,
            "{ctx}: samples should equal asammdf's; falcon read {v:?}, asammdf {:?}",
            e.iter().map(|&b| f64::from_bits(b)).collect::<Vec<_>>()
        );
    };

    match got {
        SignalValues::U8(v) => unsigned(&v.iter().map(|&x| x as u64).collect::<Vec<_>>()),
        SignalValues::U16(v) => unsigned(&v.iter().map(|&x| x as u64).collect::<Vec<_>>()),
        SignalValues::U32(v) => unsigned(&v.iter().map(|&x| x as u64).collect::<Vec<_>>()),
        SignalValues::U64(v) => unsigned(v),
        SignalValues::I8(v) => signed(&v.iter().map(|&x| x as i64).collect::<Vec<_>>()),
        SignalValues::I16(v) => signed(&v.iter().map(|&x| x as i64).collect::<Vec<_>>()),
        SignalValues::I32(v) => signed(&v.iter().map(|&x| x as i64).collect::<Vec<_>>()),
        SignalValues::I64(v) => signed(v),
        // `as f64` on an f32 is exact, and python prints an f32 as the double
        // it widens to, so this comparison is still bit-for-bit.
        SignalValues::F32(v) => float(&v.iter().map(|&x| x as f64).collect::<Vec<_>>()),
        SignalValues::F64(v) => float(v),
        SignalValues::Str(v) => {
            let e: Vec<&str> = expected.iter().map(|x| x.as_str().unwrap()).collect();
            let g: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
            assert_eq!(g, e, "{ctx}: text samples should equal asammdf's");
        }
        SignalValues::Bytes { data, width } => {
            assert_eq!(
                data.len(),
                expected.len() * *width,
                "{ctx}: falcon and asammdf should agree on the sample count"
            );
            for (i, sample) in expected.iter().enumerate() {
                let e: Vec<u8> = sample
                    .as_array()
                    .unwrap_or_else(|| panic!("{ctx}: expected a byte run per sample, got {sample}"))
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(
                    &data[i * width..(i + 1) * width],
                    e.as_slice(),
                    "{ctx}: byte sample {i} should equal asammdf's"
                );
            }
        }
        other => panic!("{ctx}: unexpected value kind {:?}", other.kind()),
    }
}

// ---------------------------------------------------------------------------
// A v3 file built from the format's block layouts
// ---------------------------------------------------------------------------

/// One channel of a synthetic file.
struct Ch {
    name: &'static str,
    /// 0 for data, 1 for the group's time channel.
    channel_type: u16,
    start_offset: u16,
    bit_count: u16,
    data_type: u16,
}

/// One channel group of a synthetic file, with its records already laid out.
struct Grp {
    record_id: u16,
    record_size: u16,
    channels: Vec<Ch>,
    /// One entry per cycle, each exactly `record_size` bytes.
    records: Vec<Vec<u8>>,
}

fn put_u16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_text(buf: &mut [u8], at: usize, len: usize, s: &str) {
    let b = s.as_bytes();
    assert!(b.len() <= len, "{s:?} does not fit in {len} bytes");
    buf[at..at + b.len()].copy_from_slice(b);
}

/// Builds a little-endian MDF 3.20 file holding one data group.
///
/// `record_id_count` is the DGBLOCK field: 0 for a sorted group, 1 for an
/// identifier before each record, 2 for one before and a copy after. Records
/// are interleaved in the order given by `order`, which names a record id per
/// record.
fn build_v3(groups: &[Grp], record_id_count: u16, order: &[u16]) -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 208];

    // Identification block. These three fields are space-padded, not
    // NUL-padded; asammdf rejects the file outright otherwise.
    put_text(&mut buf, 0, 8, "MDF     ");
    put_text(&mut buf, 8, 8, "3.20    ");
    put_text(&mut buf, 16, 8, "falcon  ");
    put_u16(&mut buf, 24, 0); // byte order: Intel
    put_u16(&mut buf, 26, 0); // float format: IEEE 754
    put_u16(&mut buf, 28, 320);

    // Header block.
    buf[64..66].copy_from_slice(b"HD");
    put_u16(&mut buf, 66, 208);
    put_u16(&mut buf, 64 + 16, groups.len() as u16);
    put_text(&mut buf, 64 + 18, 10, "01:01:2024");
    put_text(&mut buf, 64 + 28, 8, "12:00:00");

    // Channel blocks, emitted last-first so each knows where the next one is.
    let mut first_ch_of_group = Vec::new();
    for g in groups {
        let mut next = 0u32;
        for ch in g.channels.iter().rev() {
            let addr = buf.len() as u32;
            let mut cn = vec![0u8; 228];
            cn[..2].copy_from_slice(b"CN");
            put_u16(&mut cn, 2, 228);
            put_u32(&mut cn, 4, next);
            put_u16(&mut cn, 24, ch.channel_type);
            put_text(&mut cn, 26, 32, ch.name);
            put_text(&mut cn, 58, 128, "synthetic");
            put_u16(&mut cn, 186, ch.start_offset);
            put_u16(&mut cn, 188, ch.bit_count);
            put_u16(&mut cn, 190, ch.data_type);
            buf.extend_from_slice(&cn);
            next = addr;
        }
        first_ch_of_group.push(next);
    }

    // Channel group blocks, likewise.
    let mut next_cg = 0u32;
    for (i, g) in groups.iter().enumerate().rev() {
        let addr = buf.len() as u32;
        let mut cg = vec![0u8; 26];
        cg[..2].copy_from_slice(b"CG");
        put_u16(&mut cg, 2, 26);
        put_u32(&mut cg, 4, next_cg);
        put_u32(&mut cg, 8, first_ch_of_group[i]);
        put_u16(&mut cg, 16, g.record_id);
        put_u16(&mut cg, 18, g.channels.len() as u16);
        put_u16(&mut cg, 20, g.record_size);
        put_u32(&mut cg, 22, g.records.len() as u32);
        buf.extend_from_slice(&cg);
        next_cg = addr;
    }

    let dg_addr = buf.len();
    let mut dg = vec![0u8; 28];
    dg[..2].copy_from_slice(b"DG");
    put_u16(&mut dg, 2, 28);
    put_u32(&mut dg, 8, next_cg);
    put_u16(&mut dg, 20, groups.len() as u16);
    put_u16(&mut dg, 22, record_id_count);
    buf.extend_from_slice(&dg);

    let data_addr = buf.len() as u32;
    let mut taken = vec![0usize; 256];
    for &id in order {
        let g = groups
            .iter()
            .find(|g| g.record_id == id)
            .expect("the record order should name groups that exist");
        let rec = &g.records[taken[id as usize]];
        assert_eq!(rec.len(), g.record_size as usize, "record must be exactly one record long");
        taken[id as usize] += 1;
        if record_id_count > 0 {
            buf.push(id as u8);
        }
        buf.extend_from_slice(rec);
        if record_id_count == 2 {
            buf.push(id as u8);
        }
    }
    for g in groups {
        assert_eq!(
            taken[g.record_id as usize],
            g.records.len(),
            "every record of every group should appear in the stream"
        );
    }

    put_u32(&mut buf, dg_addr + 16, data_addr);
    put_u32(&mut buf, 64 + 4, dg_addr as u32);
    buf
}

/// Writes a synthetic file to `dir` and returns its path.
fn write_synthetic(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("writing the synthetic file");
    path
}

/// Pokes `bits` bits of `value` into `record` starting at bit `start`, laid out
/// the way a little-endian (Intel) v3 channel is stored.
fn poke_le(record: &mut [u8], start: usize, bits: usize, value: u64) {
    for i in 0..bits {
        if value >> i & 1 == 1 {
            record[(start + i) / 8] |= 1 << ((start + i) % 8);
        }
    }
}

/// The same, laid out the way a big-endian (Motorola) v3 channel is stored:
/// the bytes the field spans read most-significant first, with `start % 8`
/// low bits below it.
fn poke_be(record: &mut [u8], start: usize, bits: usize, value: u64) {
    let byte_offset = start / 8;
    let bit_offset = start % 8;
    let span = (bit_offset + bits).div_ceil(8);
    let shifted = (value as u128) << bit_offset;
    for i in 0..span {
        record[byte_offset + i] |= (shifted >> (8 * (span - 1 - i))) as u8;
    }
}

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
            Ch { name: "time", channel_type: 1, start_offset: 0, bit_count: 64, data_type: 3 },
            Ch { name: "LeU12", channel_type: 0, start_offset: 64, bit_count: 12, data_type: 13 },
            Ch { name: "LeI12", channel_type: 0, start_offset: 76, bit_count: 12, data_type: 14 },
            Ch { name: "BeU12", channel_type: 0, start_offset: 88, bit_count: 12, data_type: 9 },
            Ch { name: "BeI12", channel_type: 0, start_offset: 108, bit_count: 12, data_type: 10 },
            Ch { name: "BeF64", channel_type: 0, start_offset: 120, bit_count: 64, data_type: 12 },
            Ch { name: "BeF32", channel_type: 0, start_offset: 184, bit_count: 32, data_type: 11 },
            Ch { name: "Raw", channel_type: 0, start_offset: 216, bit_count: 24, data_type: 8 },
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
            Ch { name: "time_a", channel_type: 1, start_offset: 0, bit_count: 64, data_type: 3 },
            Ch { name: "A", channel_type: 0, start_offset: 64, bit_count: 32, data_type: 13 },
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
            Ch { name: "time_b", channel_type: 1, start_offset: 0, bit_count: 64, data_type: 3 },
            Ch { name: "B", channel_type: 0, start_offset: 64, bit_count: 16, data_type: 14 },
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
            Ch { name: "time", channel_type: 1, start_offset: 0, bit_count: 64, data_type: 3 },
            Ch { name: "X", channel_type: 0, start_offset, bit_count, data_type },
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

    // 4, 5 and 6 are the VAX floating point formats: real types, stored in a
    // layout that is not IEEE 754. Reading one as IEEE would give a number of
    // roughly the right magnitude and the wrong value.
    for code in [4u16, 5, 6] {
        let bytes = placement_file(64, 32, 12, code);
        let path = write_synthetic(dir.path(), &format!("vax_{code}.mdf"), &bytes);
        let file = Mdf3File::open(&path).expect("falcon should open it");
        let err = match file.values_by_name("X") {
            Ok(_) => panic!("data type {code} must be refused, not decoded as IEEE 754"),
            Err(e) => e,
        };
        assert!(
            matches!(err, falcon_mdf::Mf4Error::Unsupported { .. }),
            "the refusal should name the type, got: {err}"
        );
    }
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
