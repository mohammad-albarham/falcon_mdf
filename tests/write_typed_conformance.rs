//! What the writer emits, read back by asammdf.
//!
//! Every assertion here compares asammdf's view of a falcon-written file
//! against the values that were typed into this file — never against falcon's
//! own reader. A writer and a reader built in one repository can share a
//! misreading and round-trip happily; three silent corruption defects in this
//! repository survived exactly that arrangement.
//!
//! The two things under test are the ones a round trip through our own reader
//! would be least likely to catch:
//!
//! * **the declared type.** A `u16` channel written as eight bytes of `f64`
//!   round-trips through any reader that trusts our own `cn_data_type`. It
//!   fails here, because asammdf is asked what dtype it sees.
//! * **the conversion.** A `##CC` block we write but mis-link reads back as
//!   raw counts. asammdf applies the conversion itself, so the physical values
//!   it reports are the check.

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::{Conversion, Mf4Writer, SignalValues, TableEntry};

/// The first interpreter that exists: one beside the crate, then the shared one
/// two levels up. Agent worktrees have no `.venv` of their own.
fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// The interpreter, or a loud skip.
///
/// Returning `None` prints why. A conformance test that quietly reports success
/// while doing nothing is the failure mode this whole suite exists to avoid.
fn python() -> Option<PathBuf> {
    let Some(p) = venv_python() else {
        eprintln!("SKIPPING: no .venv/bin/python beside the crate or at ../../falcon_mdf");
        return None;
    };
    let ok = Command::new(&p)
        .args(["-c", "import asammdf"])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIPPING: asammdf is not installed in {}", p.display());
        return None;
    }
    Some(p)
}

/// Runs a script and parses its last line of stdout as JSON.
fn json(python: &Path, script: &str) -> serde_json::Value {
    let out = Command::new(python)
        .args(["-c", script])
        .output()
        .expect("failed to launch python");
    assert!(
        out.status.success(),
        "asammdf failed to read the file falcon wrote:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout.lines().last().unwrap_or_else(|| {
        panic!(
            "the reader printed nothing; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    serde_json::from_str(last).unwrap_or_else(|e| panic!("expected JSON, got {last:?}: {e}"))
}

/// Asks asammdf for every channel's dtype, unit and samples.
///
/// `raw` picks whether conversions are applied. Floats travel as the double's
/// bit pattern: a float written out in decimal and read back through a JSON
/// parser can shift by one unit in the last place, which would make this oracle
/// disagree with a correct writer over a value it got exactly right.
fn read_back(python: &Path, path: &Path, raw: bool) -> serde_json::Value {
    json(
        python,
        &format!(
            r#"
import json, struct
import numpy as np
from asammdf import MDF

m = MDF(r"{path}")
out = {{}}
for gi, g in enumerate(m.groups):
    for ci, ch in enumerate(g.channels):
        sig = m.get(group=gi, index=ci, raw={raw})
        v = np.asarray(sig.samples)
        kind = v.dtype.kind
        if kind == "S":
            values = [x.split(b"\x00")[0].decode("latin-1") for x in v.tolist()]
        elif kind in "iub":
            flat = [int(x) for x in v.ravel().tolist()]
            if v.ndim > 1:
                w = v.shape[1]
                values = [flat[i:i + w] for i in range(0, len(flat), w)]
            else:
                values = flat
        elif kind == "f":
            values = [int.from_bytes(struct.pack("<d", float(x)), "little") for x in v.tolist()]
        else:
            values = None
        out[ch.name] = {{
            "dtype": str(v.dtype),
            "kind": kind,
            "unit": sig.unit,
            "values": values,
            "timestamps": [
                int.from_bytes(struct.pack("<d", float(t)), "little")
                for t in np.asarray(sig.timestamps).tolist()
            ],
        }}
print(json.dumps(out))
m.close()
"#,
            path = path.display(),
            raw = if raw { "True" } else { "False" }
        ),
    )
}

fn ints(v: &serde_json::Value, ch: &str) -> Vec<i128> {
    v[ch]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("{ch}: asammdf reported no comparable values: {}", v[ch]))
        .iter()
        .map(|x| x.as_i64().map(i128::from).unwrap_or_else(|| x.as_u64().unwrap() as i128))
        .collect()
}

fn floats(v: &serde_json::Value, ch: &str) -> Vec<f64> {
    v[ch]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("{ch}: asammdf reported no comparable values: {}", v[ch]))
        .iter()
        .map(|x| f64::from_bits(x.as_u64().unwrap()))
        .collect()
}

fn strings(v: &serde_json::Value, ch: &str) -> Vec<String> {
    v[ch]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn temp(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join(name);
    (dir, path)
}

// ---------------------------------------------------------------------------
// Typed channels
// ---------------------------------------------------------------------------

#[test]
fn asammdf_sees_every_channel_in_the_type_we_wrote_it_in() {
    let Some(python) = python() else { return };
    let (_dir, path) = temp("typed.mf4");

    let times: Vec<f64> = (0..6).map(|i| f64::from(i) * 0.25).collect();

    // A value past 2^53, which is the whole point of writing a u64 as a u64:
    // routed through f64 it comes back as 9007199254740993 -> ...92.
    let big: Vec<u64> = vec![
        9_007_199_254_740_993,
        9_007_199_254_740_995,
        1,
        u64::MAX,
        0,
        42,
    ];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_typed("U8", "count", SignalValues::U8(vec![0, 1, 2, 254, 255, 7]))
        .unwrap();
    group
        .add_channel_typed(
            "U16",
            "rpm",
            SignalValues::U16(vec![0, 1, 65535, 32768, 1000, 7]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "U32",
            "",
            SignalValues::U32(vec![0, 1, u32::MAX, 2_000_000_000, 7, 8]),
        )
        .unwrap();
    group
        .add_channel_typed("U64", "ticks", SignalValues::U64(big.clone()))
        .unwrap();
    group
        .add_channel_typed("I8", "", SignalValues::I8(vec![-128, -1, 0, 1, 127, -7]))
        .unwrap();
    group
        .add_channel_typed(
            "I16",
            "degC",
            SignalValues::I16(vec![-32768, -1, 0, 1, 32767, -7]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I32",
            "",
            SignalValues::I32(vec![i32::MIN, -1, 0, 1, i32::MAX, -7]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I64",
            "",
            SignalValues::I64(vec![i64::MIN, -1, 0, 1, i64::MAX, -7]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "F32",
            "V",
            SignalValues::F32(vec![0.0, -1.5, 3.4e38, -1.0 / 3.0, 1.0, 2.5]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "F64",
            "km/h",
            SignalValues::F64(vec![0.0, -1.5, 1e300, 1.0 / 3.0, 1.0, 2.5]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "Label",
            "",
            SignalValues::Str(
                ["idle", "run", "", "stop", "fault", "idle"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
        )
        .unwrap();
    group
        .add_channel_typed(
            "Payload",
            "",
            SignalValues::Bytes {
                data: (0..18u8).collect(),
                width: 3,
            },
        )
        .unwrap();
    writer.write_to_file(&path).unwrap();

    let got = read_back(&python, &path, true);

    // The declared type, as asammdf sees it. This is the assertion that fails
    // if a channel is quietly widened to f64.
    for (name, want) in [
        ("U8", "uint8"),
        ("U16", "uint16"),
        ("U32", "uint32"),
        ("U64", "uint64"),
        ("I8", "int8"),
        ("I16", "int16"),
        ("I32", "int32"),
        ("I64", "int64"),
        ("F32", "float32"),
        ("F64", "float64"),
    ] {
        assert_eq!(
            got[name]["dtype"].as_str().unwrap(),
            want,
            "asammdf should see {name} as {want}, not as something wider"
        );
    }

    // The values themselves.
    assert_eq!(ints(&got, "U8"), vec![0, 1, 2, 254, 255, 7]);
    assert_eq!(ints(&got, "U16"), vec![0, 1, 65535, 32768, 1000, 7]);
    assert_eq!(
        ints(&got, "U32"),
        vec![0, 1, i128::from(u32::MAX), 2_000_000_000, 7, 8]
    );
    assert_eq!(
        ints(&got, "U64"),
        big.iter().map(|&x| i128::from(x)).collect::<Vec<_>>(),
        "a u64 past 2^53 must survive; through f64 it would not"
    );
    assert_eq!(ints(&got, "I8"), vec![-128, -1, 0, 1, 127, -7]);
    assert_eq!(ints(&got, "I16"), vec![-32768, -1, 0, 1, 32767, -7]);
    assert_eq!(
        ints(&got, "I32"),
        vec![i128::from(i32::MIN), -1, 0, 1, i128::from(i32::MAX), -7]
    );
    assert_eq!(
        ints(&got, "I64"),
        vec![i128::from(i64::MIN), -1, 0, 1, i128::from(i64::MAX), -7]
    );

    let f32s = floats(&got, "F32");
    let want32: Vec<f64> = [0.0f32, -1.5, 3.4e38, -1.0 / 3.0, 1.0, 2.5]
        .iter()
        .map(|&x| f64::from(x))
        .collect();
    assert_eq!(f32s, want32, "float32 samples should widen exactly");
    assert_eq!(
        floats(&got, "F64"),
        vec![0.0, -1.5, 1e300, 1.0 / 3.0, 1.0, 2.5]
    );

    assert_eq!(
        strings(&got, "Label"),
        vec!["idle", "run", "", "stop", "fault", "idle"],
        "a fixed-length string channel should read back with its padding stripped"
    );
    assert_eq!(
        got["Payload"]["values"],
        serde_json::json!([[0, 1, 2], [3, 4, 5], [6, 7, 8], [9, 10, 11], [12, 13, 14], [15, 16, 17]]),
        "a byte-array channel should read back three bytes per sample"
    );

    // Units and timestamps, which travel on different links from the samples.
    for (name, unit) in [
        ("U8", "count"),
        ("U16", "rpm"),
        ("U64", "ticks"),
        ("I16", "degC"),
        ("F32", "V"),
        ("F64", "km/h"),
        ("U32", ""),
    ] {
        assert_eq!(
            got[name]["unit"].as_str().unwrap(),
            unit,
            "{name}'s unit should survive"
        );
    }
    let stamps: Vec<f64> = got["U8"]["timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| f64::from_bits(x.as_u64().unwrap()))
        .collect();
    assert_eq!(stamps, times, "the time axis should read back unchanged");
}

#[test]
fn a_sample_kind_with_no_record_layout_is_refused_by_name() {
    let times = vec![0.0, 1.0];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();

    // Variable-length byte runs need VLSD, which this writer does not emit.
    let err = group
        .add_channel_typed(
            "Ragged",
            "",
            SignalValues::VarBytes {
                data: vec![1, 2, 3],
                starts: vec![0, 1, 3],
            },
        )
        .unwrap_err();
    assert!(matches!(err, falcon_mdf::Mf4Error::WriteError { .. }), "{err}");
    assert!(
        err.to_string().contains("record layout"),
        "the refusal should say why: {err}"
    );

    let err = group
        .add_channel_typed(
            "Wave",
            "",
            SignalValues::Complex {
                re: vec![1.0, 2.0],
                im: vec![0.0, 1.0],
            },
        )
        .unwrap_err();
    assert!(matches!(err, falcon_mdf::Mf4Error::WriteError { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

#[test]
fn asammdf_applies_the_conversions_we_write() {
    let Some(python) = python() else { return };
    let (_dir, path) = temp("conversions.mf4");

    let times: Vec<f64> = (0..5).map(f64::from).collect();
    let raw: Vec<f64> = vec![0.0, 1.0, 2.0, 3.0, 4.0];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_typed_with(
            "Lin",
            "km/h",
            SignalValues::U16(vec![0, 1, 2, 3, 4]),
            None,
            Some(Conversion::Linear {
                offset: -1.25,
                factor: 2.5,
            }),
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "Rat",
            "",
            SignalValues::F64(raw.clone()),
            None,
            Some(Conversion::Rational {
                coefficients: [1.0, 2.0, 3.0, 0.0, 1.0, 4.0],
            }),
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "Alg",
            "",
            SignalValues::F64(raw.clone()),
            None,
            Some(Conversion::Algebraic {
                formula: "3*X**2 + 2*X - 1".to_string(),
                expr: falcon_mdf::blocks::formula::Expr::parse("3*X**2 + 2*X - 1").unwrap(),
            }),
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "Interp",
            "",
            SignalValues::F64(raw.clone()),
            None,
            Some(Conversion::TableInterpolated {
                keys: vec![0.0, 2.0, 4.0],
                values: vec![0.0, 20.0, 30.0],
            }),
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "Lookup",
            "",
            SignalValues::F64(raw.clone()),
            None,
            Some(Conversion::TableLookup {
                keys: vec![0.0, 2.0, 4.0],
                values: vec![0.0, 20.0, 30.0],
            }),
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "State",
            "",
            SignalValues::U8(vec![0, 1, 2, 3, 4]),
            None,
            Some(Conversion::ValueToText {
                keys: vec![0.0, 1.0, 2.0],
                entries: vec![
                    TableEntry::Text("idle".into()),
                    TableEntry::Text("run".into()),
                    TableEntry::Text("stop".into()),
                ],
                default: Some(TableEntry::Text("unknown".into())),
            }),
        )
        .unwrap();
    writer.write_to_file(&path).unwrap();

    // Raw first: the stored counts must be untouched by the conversion.
    let stored = read_back(&python, &path, true);
    assert_eq!(ints(&stored, "Lin"), vec![0, 1, 2, 3, 4]);
    assert_eq!(
        stored["Lin"]["dtype"].as_str().unwrap(),
        "uint16",
        "a converted channel keeps its stored type"
    );

    // Then physical: asammdf applies the CC block we wrote.
    let phys = read_back(&python, &path, false);
    assert_eq!(
        floats(&phys, "Lin"),
        vec![-1.25, 1.25, 3.75, 6.25, 8.75],
        "linear conversion"
    );
    assert_eq!(
        floats(&phys, "Rat"),
        raw.iter()
            .map(|&x| (1.0 * x * x + 2.0 * x + 3.0) / (0.0 * x * x + 1.0 * x + 4.0))
            .collect::<Vec<_>>(),
        "rational conversion"
    );
    assert_eq!(
        floats(&phys, "Alg"),
        raw.iter().map(|&x| 3.0 * x * x + 2.0 * x - 1.0).collect::<Vec<_>>(),
        "algebraic conversion"
    );
    assert_eq!(
        floats(&phys, "Interp"),
        vec![0.0, 10.0, 20.0, 25.0, 30.0],
        "tabular with interpolation"
    );
    assert_eq!(
        floats(&phys, "Lookup"),
        vec![0.0, 0.0, 20.0, 20.0, 30.0],
        "tabular without interpolation"
    );
    assert_eq!(
        strings(&phys, "State"),
        vec!["idle", "run", "stop", "unknown", "unknown"],
        "value-to-text table, including its default"
    );
    assert_eq!(
        phys["Lin"]["unit"].as_str().unwrap(),
        "km/h",
        "the unit stays on the channel, not on the conversion"
    );
}

#[test]
fn a_channel_whose_values_are_already_physical_carries_no_conversion_block() {
    let Some(python) = python() else { return };
    let (_dir, path) = temp("identity.mf4");

    let times = vec![0.0, 1.0];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    // An identity linear rule is what a writer emits when it has nothing to
    // say; it should produce a null link rather than a block that does nothing.
    group
        .add_channel_typed_with(
            "Plain",
            "V",
            SignalValues::F64(vec![1.5, 2.5]),
            None,
            Some(Conversion::Linear {
                offset: 0.0,
                factor: 1.0,
            }),
        )
        .unwrap();
    writer.write_to_file(&path).unwrap();

    let phys = read_back(&python, &path, false);
    assert_eq!(floats(&phys, "Plain"), vec![1.5, 2.5]);

    let has_cc = json(
        &python,
        &format!(
            r#"
import json
from asammdf import MDF
m = MDF(r"{path}")
ch = m.groups[0].channels[1]
print(json.dumps({{"name": ch.name, "cc": ch.conversion is not None}}))
m.close()
"#,
            path = path.display()
        ),
    );
    assert_eq!(has_cc["name"].as_str().unwrap(), "Plain");
    assert!(
        !has_cc["cc"].as_bool().unwrap(),
        "an identity conversion should be written as no block at all"
    );
}

#[test]
fn a_conversion_this_writer_cannot_express_is_refused_by_name() {
    let times = vec![0.0, 1.0];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();

    // Text-keyed and bitfield tables have no writer support. Dropping the
    // conversion instead would write raw counts under a physical unit.
    for conversion in [
        Conversion::TextToValue {
            keys: vec!["a".into()],
            values: vec![1.0],
            default: None,
        },
        Conversion::Bitfield {
            masks: vec![0xF],
            entries: Vec::new(),
        },
        Conversion::RangeToText {
            lower: vec![0.0],
            upper: vec![1.0],
            entries: vec![TableEntry::Text("low".into())],
            default: None,
        },
    ] {
        let err = group
            .add_channel_typed_with(
                "X",
                "V",
                SignalValues::F64(vec![1.0, 2.0]),
                None,
                Some(conversion),
            )
            .unwrap_err();
        assert!(matches!(err, falcon_mdf::Mf4Error::WriteError { .. }), "{err}");
        assert!(
            err.to_string().contains("cannot express"),
            "the refusal should say so plainly: {err}"
        );
    }

    // A nested conversion inside a text table is refused for the same reason.
    let err = group
        .add_channel_typed_with(
            "Y",
            "",
            SignalValues::F64(vec![1.0, 2.0]),
            None,
            Some(Conversion::ValueToText {
                keys: vec![0.0],
                entries: vec![TableEntry::Nested(Box::new(Conversion::Linear {
                    offset: 0.0,
                    factor: 2.0,
                }))],
                default: None,
            }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("nested"), "{err}");
}
