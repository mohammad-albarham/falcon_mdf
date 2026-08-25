//! Shared fixtures for the MDF 3.x conformance tests.
//!
//! Two things live here because both the record-decoding tests and the
//! conversion tests need them, and a second copy of either would be a second
//! thing to get wrong:
//!
//! * **the oracle** — running asammdf over a file and reporting what it made of
//!   it, so that no expected value in either suite comes from this crate;
//! * **a v3 file builder** — laying out ID/HD/DG/CG/CN/CC/TX blocks and a
//!   record stream from the format's own field offsets. asammdf's v3 writer
//!   cannot produce an unsorted file, a Motorola channel, or a polynomial,
//!   exponential, logarithmic or text-table conversion; those fixtures have no
//!   other way to exist. asammdf still *reads* what is built here, so it is
//!   still the oracle for the values.

// Each test binary uses a different part of this module, so anything one of
// them does not call would otherwise warn there.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::SignalValues;

// ---------------------------------------------------------------------------
// asammdf, as the oracle
// ---------------------------------------------------------------------------

/// Locates the virtualenv python that has asammdf installed.
pub fn venv_python() -> PathBuf {
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
pub fn python_json(script: &str) -> serde_json::Value {
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
/// `raw=True` keeps conversions out of it: these are the values as stored.
pub fn asammdf_raw_samples(path: &Path) -> serde_json::Value {
    asammdf_samples(path, true)
}

/// Asks asammdf what physical samples a v3 file holds, channel by channel —
/// the stored values with each channel's conversion applied.
pub fn asammdf_physical_samples(path: &Path) -> serde_json::Value {
    asammdf_samples(path, false)
}

/// Asks asammdf what a v3 file holds, channel by channel.
pub fn asammdf_samples(path: &Path, raw: bool) -> serde_json::Value {
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
        vals = m.get(group=gi, index=ci, raw={raw}, samples_only=True)[0]
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
        path = path.display(),
        raw = if raw { "True" } else { "False" }
    ))
}

/// Compares falcon's decoded samples with what asammdf reported for the same
/// channel, sample for sample.
pub fn assert_same_samples(ctx: &str, got: &SignalValues, want: &serde_json::Value) {
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
pub struct Ch {
    pub name: &'static str,
    /// 0 for data, 1 for the group's time channel.
    pub channel_type: u16,
    pub start_offset: u16,
    pub bit_count: u16,
    pub data_type: u16,
    /// The conversion block to attach, if any.
    pub conversion: Option<Cc>,
}

impl Ch {
    /// A channel with no conversion, whose raw values are already physical.
    pub fn new(
        name: &'static str,
        channel_type: u16,
        start_offset: u16,
        bit_count: u16,
        data_type: u16,
    ) -> Self {
        Self {
            name,
            channel_type,
            start_offset,
            bit_count,
            data_type,
            conversion: None,
        }
    }

    /// The same, carrying a conversion.
    pub fn with(mut self, cc: Cc) -> Self {
        self.conversion = Some(cc);
        self
    }
}

/// A conversion block to emit for a channel.
///
/// The variants name the `CCBLOCK` types by their codes, because that is what
/// the file records and what asammdf reports back.
pub enum Cc {
    /// Type 0. Stored offset-first, which this builder does for you.
    Linear { a: f64, b: f64 },
    /// Type 1: tabular with interpolation, as `(raw, physical)` pairs.
    Tabi(Vec<(f64, f64)>),
    /// Type 2: tabular without interpolation.
    Tab(Vec<(f64, f64)>),
    /// Type 6: polynomial, `P1` to `P6`.
    Poly([f64; 6]),
    /// Type 7: exponential, `P1` to `P7`.
    Expo([f64; 7]),
    /// Type 8: logarithmic, `P1` to `P7`.
    Logh([f64; 7]),
    /// Type 9: rational, `P1` to `P6`.
    Rat([f64; 6]),
    /// Type 10: an algebraic formula.
    Formula(&'static str),
    /// Type 11: value to text, as `(raw, label)` pairs. The label is written
    /// into the block itself, in a 32-byte field.
    TextTable(Vec<(f64, &'static str)>),
    /// Type 12: value range to text, as `(lower, upper, label)` triples plus
    /// the label for a value in no range. Every label goes in a `TX` block.
    RangeTable {
        ranges: Vec<(f64, f64, &'static str)>,
        default: &'static str,
    },
    /// A block written exactly as given, for types the format does not define.
    Raw { code: u16, count: u16, params: Vec<u8> },
}

impl Cc {
    /// The type code this conversion writes into the block.
    fn code(&self) -> u16 {
        match self {
            Cc::Linear { .. } => 0,
            Cc::Tabi(_) => 1,
            Cc::Tab(_) => 2,
            Cc::Poly(_) => 6,
            Cc::Expo(_) => 7,
            Cc::Logh(_) => 8,
            Cc::Rat(_) => 9,
            Cc::Formula(_) => 10,
            Cc::TextTable(_) => 11,
            Cc::RangeTable { .. } => 12,
            Cc::Raw { code, .. } => *code,
        }
    }
}

/// Appends a `TX` block and returns its address.
fn push_tx(buf: &mut Vec<u8>, text: &str) -> u32 {
    let addr = buf.len() as u32;
    let len = 4 + text.len() + 1;
    let mut tx = vec![0u8; len];
    tx[..2].copy_from_slice(b"TX");
    put_u16(&mut tx, 2, len as u16);
    tx[4..4 + text.len()].copy_from_slice(text.as_bytes());
    buf.extend_from_slice(&tx);
    addr
}

/// Appends a conversion block, plus any `TX` blocks it references, and returns
/// the conversion block's address.
fn push_cc(buf: &mut Vec<u8>, cc: &Cc) -> u32 {
    // Referenced text has to exist before the block that links to it, since
    // this builder only ever appends.
    let text_addrs: Vec<u32> = match cc {
        Cc::RangeTable { ranges, default } => {
            let mut a = vec![push_tx(buf, default)];
            a.extend(ranges.iter().map(|(_, _, t)| push_tx(buf, t)));
            a
        }
        _ => Vec::new(),
    };

    let mut params: Vec<u8> = Vec::new();
    let push_f = |p: &mut Vec<u8>, v: f64| p.extend_from_slice(&v.to_le_bytes());
    let count: u16 = match cc {
        Cc::Linear { a, b } => {
            push_f(&mut params, *b);
            push_f(&mut params, *a);
            2
        }
        Cc::Tabi(pairs) | Cc::Tab(pairs) => {
            for (raw, phys) in pairs {
                push_f(&mut params, *raw);
                push_f(&mut params, *phys);
            }
            pairs.len() as u16
        }
        Cc::Poly(p) | Cc::Rat(p) => {
            for v in p {
                push_f(&mut params, *v);
            }
            6
        }
        Cc::Expo(p) | Cc::Logh(p) => {
            for v in p {
                push_f(&mut params, *v);
            }
            7
        }
        Cc::Formula(text) => {
            params.extend_from_slice(text.as_bytes());
            params.push(0);
            text.len() as u16
        }
        Cc::TextTable(pairs) => {
            for (raw, label) in pairs {
                push_f(&mut params, *raw);
                let mut field = [0u8; 32];
                field[..label.len()].copy_from_slice(label.as_bytes());
                params.extend_from_slice(&field);
            }
            pairs.len() as u16
        }
        Cc::RangeTable { ranges, .. } => {
            // The first entry is the default: two unused bounds and the
            // default text's address.
            push_f(&mut params, 0.0);
            push_f(&mut params, 0.0);
            params.extend_from_slice(&text_addrs[0].to_le_bytes());
            for (i, (lower, upper, _)) in ranges.iter().enumerate() {
                push_f(&mut params, *lower);
                push_f(&mut params, *upper);
                params.extend_from_slice(&text_addrs[i + 1].to_le_bytes());
            }
            (ranges.len() + 1) as u16
        }
        Cc::Raw { count, params: p, .. } => {
            params.extend_from_slice(p);
            *count
        }
    };

    let addr = buf.len() as u32;
    let len = 46 + params.len();
    let mut cc_bytes = vec![0u8; 46];
    cc_bytes[..2].copy_from_slice(b"CC");
    put_u16(&mut cc_bytes, 2, len as u16);
    put_text(&mut cc_bytes, 22, 20, "u");
    put_u16(&mut cc_bytes, 42, cc.code());
    put_u16(&mut cc_bytes, 44, count);
    cc_bytes.extend_from_slice(&params);
    buf.extend_from_slice(&cc_bytes);
    addr
}

/// One channel group of a synthetic file, with its records already laid out.
pub struct Grp {
    pub record_id: u16,
    pub record_size: u16,
    pub channels: Vec<Ch>,
    /// One entry per cycle, each exactly `record_size` bytes.
    pub records: Vec<Vec<u8>>,
}

pub fn put_u16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put_text(buf: &mut [u8], at: usize, len: usize, s: &str) {
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
pub fn build_v3(groups: &[Grp], record_id_count: u16, order: &[u16]) -> Vec<u8> {
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
            let cc_addr = match &ch.conversion {
                Some(cc) => push_cc(&mut buf, cc),
                None => 0,
            };
            let addr = buf.len() as u32;
            let mut cn = vec![0u8; 228];
            cn[..2].copy_from_slice(b"CN");
            put_u16(&mut cn, 2, 228);
            put_u32(&mut cn, 4, next);
            put_u32(&mut cn, 8, cc_addr);
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
pub fn write_synthetic(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("writing the synthetic file");
    path
}

/// Pokes `bits` bits of `value` into `record` starting at bit `start`, laid out
/// the way a little-endian (Intel) v3 channel is stored.
pub fn poke_le(record: &mut [u8], start: usize, bits: usize, value: u64) {
    for i in 0..bits {
        if value >> i & 1 == 1 {
            record[(start + i) / 8] |= 1 << ((start + i) % 8);
        }
    }
}

/// The same, laid out the way a big-endian (Motorola) v3 channel is stored:
/// the bytes the field spans read most-significant first, with `start % 8`
/// low bits below it.
pub fn poke_be(record: &mut [u8], start: usize, bits: usize, value: u64) {
    let byte_offset = start / 8;
    let bit_offset = start % 8;
    let span = (bit_offset + bits).div_ceil(8);
    let shifted = (value as u128) << bit_offset;
    for i in 0..span {
        record[byte_offset + i] |= (shifted >> (8 * (span - 1 - i))) as u8;
    }
}

