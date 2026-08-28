//! MDF 3.x conversions, checked against asammdf converting the same bytes.
//!
//! Every expected value here comes from asammdf reading the file falcon reads,
//! never from this crate's own arithmetic. Where asammdf's v3 writer cannot
//! produce a conversion — it never emits a polynomial, exponential,
//! logarithmic or value-to-text block — the file is built from the format's
//! block layouts and asammdf still reads it.
//!
//! Tolerance: **exact equality**, compared as bit patterns. Every rule here is
//! evaluated as the same operations in the same order asammdf uses, in `f64`,
//! so there is nothing for a tolerance to absorb. The two rules that call libm
//! — exponential and logarithmic — are the exception and are stated as such at
//! their test; everything else has to agree to the last bit or it is wrong.

#![cfg(feature = "mdf3")]

use std::path::{Path, PathBuf};

use falcon_mdf::mdf3::conversions::{Mdf3Conversion, Mdf3ConversionOutput};
use falcon_mdf::mdf3::Mdf3File;
use falcon_mdf::SignalValues;

mod common;
use common::{
    asammdf_physical_samples, assert_same_samples, build_v3, python_json, write_synthetic, Cc, Ch,
    Grp,
};

/// The raw samples every synthetic fixture below carries: 0 to 9 as a double,
/// which is what asammdf's own conversion probes use.
const RAW: [f64; 10] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];

/// Builds a one-group file whose single data channel carries `cc`.
///
/// The channel is a double so that the raw values reach the conversion
/// unrounded; what is under test is the rule, not the record layout.
fn conversion_file(dir: &Path, name: &str, cc: Cc) -> PathBuf {
    let records = RAW
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let mut r = vec![0u8; 16];
            r[0..8].copy_from_slice(&(i as f64 * 0.1).to_le_bytes());
            r[8..16].copy_from_slice(&v.to_le_bytes());
            r
        })
        .collect();
    let grp = Grp {
        record_id: 0,
        record_size: 16,
        channels: vec![
            Ch::new("time", 1, 0, 64, 3),
            Ch::new("X", 0, 64, 64, 3).with(cc),
        ],
        records,
    };
    write_synthetic(dir, name, &build_v3(&[grp], 0, &[0; 10]))
}

/// Reads a channel's physical values as `f64`, failing if it did not convert
/// to numbers.
fn physical(file: &Mdf3File, name: &str) -> Vec<f64> {
    match file
        .physical_by_name(name)
        .unwrap_or_else(|e| panic!("converting {name}: {e}"))
    {
        SignalValues::F64(v) => v,
        other => panic!("{name} should convert to f64, got {:?}", other.kind()),
    }
}

/// Reads a channel's physical values as text.
fn labels(file: &Mdf3File, name: &str) -> Vec<String> {
    match file
        .physical_by_name(name)
        .unwrap_or_else(|e| panic!("converting {name}: {e}"))
    {
        SignalValues::Str(v) => v,
        other => panic!("{name} should convert to text, got {:?}", other.kind()),
    }
}

/// Checks falcon's physical values against asammdf's, channel for channel.
fn assert_matches_asammdf(path: &Path, file: &Mdf3File) {
    if !common::asammdf_available() {
        eprintln!("skipping: asammdf not available");
        return;
    }
    let expected = asammdf_physical_samples(path);
    let expected = expected.as_array().expect("a channel list");
    assert!(!expected.is_empty(), "the oracle should report channels");
    for want in expected {
        let name = want["name"].as_str().unwrap();
        let got = file
            .physical_by_name(name)
            .unwrap_or_else(|e| panic!("converting {name}: {e}"));
        assert_same_samples(name, &got, want);
    }
}

// ---------------------------------------------------------------------------
// Conversions asammdf's writer can produce
// ---------------------------------------------------------------------------

/// Writes a v3 file with asammdf, one channel per conversion it can emit.
///
/// Verified on this machine that these come back as v3 types 0 (linear), 9
/// (rational), 10 (formula), 1 (tabular interpolated), 2 (tabular) and 12
/// (value range to text) — asammdf routes both of its text tables through
/// type 12.
fn asammdf_written_conversions(dir: &Path) -> PathBuf {
    let path = dir.join("written_conversions.mdf");
    python_json(&format!(
        r#"
import json
import numpy as np
from asammdf import MDF, Signal

t = np.arange(0.0, 1.0, 0.1)
x = np.arange(10, dtype=np.float64)

cases = {{
  "Plain": None,
  "Lin":   {{"a": 2.5, "b": -1.25}},
  "Rat":   {{"P1": 1.0, "P2": 2.0, "P3": 3.0, "P4": 0.0, "P5": 1.0, "P6": 4.0}},
  "Form":  {{"formula": "3*X**2 + 2*X - 1"}},
  "Tabi":  {{"raw_0": 0.0, "phys_0": 0.0, "raw_1": 3.0, "phys_1": 30.0,
             "raw_2": 9.0, "phys_2": 45.0, "interpolation": True}},
  "Tab":   {{"raw_0": 0.0, "phys_0": 0.0, "raw_1": 3.0, "phys_1": 30.0,
             "raw_2": 9.0, "phys_2": 45.0, "interpolation": False}},
  "Rtabx": {{"lower_0": 0.0, "upper_0": 2.0, "text_0": b"low",
             "lower_1": 2.0, "upper_1": 5.0, "text_1": b"mid",
             "lower_2": 5.0, "upper_2": 9.0, "text_2": b"high",
             "default_addr": b"none"}},
}}
sigs = [
    Signal(samples=x.copy(), timestamps=t, name=n, unit="u", conversion=c)
    for n, c in cases.items()
]
m = MDF(version="3.30")
m.append(sigs)
m.save(r"{path}", overwrite=True)
m.close()
print(json.dumps("written"))
"#,
        path = path.display()
    ));
    path
}

#[test]
fn conversions_asammdf_wrote_convert_to_the_same_values() {
    if !common::asammdf_available() {
        eprintln!("skipping: asammdf not available");
        return;
    }
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = asammdf_written_conversions(dir.path());
    let file = Mdf3File::open(&path).expect("falcon should open it");

    // The types the file actually holds, so a change in what asammdf emits
    // shows up here rather than quietly narrowing what this test covers.
    let types: Vec<u16> = ["Plain", "Lin", "Rat", "Form", "Tabi", "Tab", "Rtabx"]
        .iter()
        .map(|n| {
            let (g, c, i) = index_of(&file, n);
            match file.channel_conversion(g, c, i).unwrap() {
                Mdf3Conversion::None => 65535,
                Mdf3Conversion::Linear { .. } => 0,
                Mdf3Conversion::TabularInterpolated { .. } => 1,
                Mdf3Conversion::Tabular { .. } => 2,
                Mdf3Conversion::Rational { .. } => 9,
                Mdf3Conversion::Formula { .. } => 10,
                Mdf3Conversion::TextRangeTable { .. } => 12,
                other => panic!("{n} parsed as an unexpected rule: {other:?}"),
            }
        })
        .collect();
    assert_eq!(
        types,
        vec![65535, 0, 9, 10, 1, 2, 12],
        "the fixture should cover linear, rational, formula, both tabular \
         forms and the range table"
    );

    assert_matches_asammdf(&path, &file);

    // And the values themselves, so the comparison above is not two readers
    // agreeing on nothing.
    assert_eq!(physical(&file, "Lin")[..3], [-1.25, 1.25, 3.75]);
    assert_eq!(physical(&file, "Tabi")[..4], [0.0, 10.0, 20.0, 30.0]);
    assert_eq!(physical(&file, "Tab")[..4], [0.0, 0.0, 30.0, 30.0]);
    assert_eq!(physical(&file, "Form")[..3], [-1.0, 4.0, 15.0]);
}

#[test]
fn a_channel_without_a_conversion_keeps_its_stored_type() {
    if !common::asammdf_available() {
        eprintln!("skipping: asammdf not available");
        return;
    }
    // An identity conversion must not push an integer channel through f64.
    // asammdf writes the raw dtype back for one, and so does falcon.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = asammdf_written_conversions(dir.path());
    let file = Mdf3File::open(&path).expect("falcon should open it");

    let (g, c, i) = index_of(&file, "Plain");
    assert!(file.channel_conversion(g, c, i).unwrap().is_identity());
    assert_eq!(
        file.channel_physical(g, c, i).unwrap(),
        file.channel_values(g, c, i).unwrap(),
        "an identity conversion should return exactly the raw samples"
    );
}

/// The three indices of a named channel.
fn index_of(file: &Mdf3File, name: &str) -> (usize, usize, usize) {
    for (g, dg) in file.data_groups().iter().enumerate() {
        for (c, cg) in dg.channel_groups.iter().enumerate() {
            if let Some(i) = cg.channels.iter().position(|ch| ch.name == name) {
                return (g, c, i);
            }
        }
    }
    panic!("no channel {name}");
}

// ---------------------------------------------------------------------------
// Conversions asammdf's writer cannot produce
// ---------------------------------------------------------------------------

#[test]
fn a_polynomial_conversion_matches_asammdf_in_both_its_forms() {
    let dir = tempfile::tempdir().expect("a temp dir");

    // The long form: (P2 - P4·(X - P5 - P6)) / (P3·(X - P5 - P6) - P1).
    let long = conversion_file(
        dir.path(),
        "poly_long.mdf",
        Cc::Poly([1.0, 2.0, 3.0, 4.0, 0.5, 0.25]),
    );
    let file = Mdf3File::open(&long).expect("falcon should open it");
    assert_matches_asammdf(&long, &file);
    let got = physical(&file, "X");
    for (i, &x) in RAW.iter().enumerate() {
        let s = x - 0.5 - 0.25;
        assert_eq!(got[i], (2.0 - 4.0 * s) / (3.0 * s - 1.0), "sample {i}");
    }

    // The short form, taken when P2, P3, P5 and P6 are all zero: P4·X / P1.
    let short = conversion_file(
        dir.path(),
        "poly_short.mdf",
        Cc::Poly([4.0, 0.0, 0.0, 10.0, 0.0, 0.0]),
    );
    let file = Mdf3File::open(&short).expect("falcon should open it");
    assert_matches_asammdf(&short, &file);
    assert_eq!(physical(&file, "X")[..3], [0.0, 2.5, 5.0]);
}

#[test]
fn exponential_and_logarithmic_conversions_match_asammdf() {
    // These two are the only rules here that reach libm, so they are the only
    // ones where falcon's `exp`/`ln` and numpy's could in principle differ in
    // the last bit. On this machine they do not, so the comparison is still an
    // equality; if that ever changes, this is the test that will say so, and
    // the fix is a stated relative tolerance rather than a shrug.
    let dir = tempfile::tempdir().expect("a temp dir");

    // P4 == 0 selects f(((X - P7)·P6 - P3) / P1) / P2.
    for (name, cc) in [
        ("expo_a.mdf", Cc::Expo([2.0, 4.0, 1.0, 0.0, 0.0, 3.0, 1.0])),
        (
            "logh_a.mdf",
            Cc::Logh([2.0, 4.0, 1.0, 0.0, 0.0, 3.0, -20.0]),
        ),
    ] {
        let path = conversion_file(dir.path(), name, cc);
        let file = Mdf3File::open(&path).expect("falcon should open it");
        assert_matches_asammdf(&path, &file);
    }

    // P1 == 0 selects f((P3 / (X - P7) - P6) / P4) / P5.
    for (name, cc) in [
        ("expo_b.mdf", Cc::Expo([0.0, 0.0, 6.0, 2.0, 3.0, 1.0, -0.5])),
        (
            "logh_b.mdf",
            Cc::Logh([0.0, 0.0, 60.0, 2.0, 3.0, 1.0, -0.5]),
        ),
    ] {
        let path = conversion_file(dir.path(), name, cc);
        let file = Mdf3File::open(&path).expect("falcon should open it");
        assert_matches_asammdf(&path, &file);
    }
}

#[test]
fn a_value_to_text_table_matches_asammdf() {
    // Conversion type 11 keeps its labels inside the block, in 32-byte fields,
    // rather than behind TX links. asammdf reads type 11 but writes its value
    // tables as type 12, so this fixture is built here.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "text_table.mdf",
        Cc::TextTable(vec![(1.0, "one"), (3.0, "three"), (5.0, "five")]),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");

    assert_eq!(
        labels(&file, "X"),
        vec!["", "one", "", "three", "", "five", "", "", "", ""],
        "an exact match gets its label and everything else gets none"
    );
    assert_matches_asammdf(&path, &file);
}

#[test]
fn a_value_range_table_puts_a_shared_boundary_in_neither_range() {
    // This is v3's rule and it is not v4's. See the report and
    // `Mdf3Conversion::convert_text`.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "range_table.mdf",
        Cc::RangeTable {
            ranges: vec![(0.0, 2.0, "low"), (2.0, 5.0, "mid"), (5.0, 9.0, "high")],
            default: "none",
        },
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");

    assert_eq!(
        labels(&file, "X"),
        vec!["low", "low", "none", "mid", "mid", "none", "high", "high", "high", "high"],
        "2 and 5 end one range and start the next, so they belong to neither; \
         9 ends the last range and is not shared, so it is inside it"
    );
    assert_matches_asammdf(&path, &file);
}

#[test]
fn a_tabular_conversion_interpolates_exactly_as_asammdf_does() {
    // The two expressions `slope*(x - x0) + y0` and `y0 + t*(y1 - y0)` are the
    // same number in exact arithmetic and not always the same f64. This fixture
    // uses keys and values that make them differ, so the test would fail if the
    // wrong one were used.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "interp.mdf",
        Cc::Tabi(vec![
            (0.0, 0.1),
            (3.0, 0.30000000000000004),
            (7.0, 1.0 / 3.0),
            (9.0, 2.0 / 3.0),
        ]),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    assert_matches_asammdf(&path, &file);
}

// ---------------------------------------------------------------------------
// What must fail rather than return a number
// ---------------------------------------------------------------------------

#[test]
fn a_conversion_type_this_build_does_not_evaluate_is_refused_by_name() {
    // No silent identity fallback: raw counts returned as physical values would
    // be a wrong measurement rather than a missing one.
    let dir = tempfile::tempdir().expect("a temp dir");
    for code in [3u16, 4, 5, 13, 100] {
        let path = conversion_file(
            dir.path(),
            &format!("unknown_{code}.mdf"),
            Cc::Raw {
                code,
                count: 0,
                params: Vec::new(),
            },
        );
        let file = Mdf3File::open(&path).expect("falcon should open it");
        let err = match file.physical_by_name("X") {
            Ok(v) => panic!(
                "conversion type {code} must be refused, got {} values back",
                v.len()
            ),
            Err(e) => e,
        };
        assert!(
            matches!(err, falcon_mdf::Mf4Error::Unsupported { .. }),
            "type {code} should be refused by name, got: {err}"
        );
        assert!(
            err.to_string().contains(&code.to_string()),
            "the refusal should name the type, got: {err}"
        );

        // The raw samples are still readable — what is refused is calling them
        // physical values.
        assert_eq!(file.values_by_name("X").unwrap().len(), RAW.len());
    }
}

#[test]
fn an_exponential_conversion_that_names_neither_form_is_refused() {
    // With both P1 and P4 non-zero the block says nothing about which of the
    // two formulas applies. asammdf raises here too.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "expo_ambiguous.mdf",
        Cc::Expo([2.0, 1.0, 1.0, 3.0, 1.0, 1.0, 0.0]),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let err = match file.physical_by_name("X") {
        Ok(_) => panic!("an exponential block naming neither form must be refused"),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidConversion { .. }),
        "got: {err}"
    );
}

#[test]
fn a_table_whose_keys_do_not_ascend_is_refused_rather_than_binary_searched() {
    // Every lookup here is a binary search, and a binary search over unsorted
    // keys returns a wrong entry rather than no entry. asammdf has the same
    // requirement and does not check it.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "unsorted_table.mdf",
        Cc::Tabi(vec![(0.0, 0.0), (9.0, 90.0), (3.0, 30.0)]),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let err = match file.physical_by_name("X") {
        Ok(_) => panic!("a table with descending keys must be refused"),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidConversion { .. }),
        "got: {err}"
    );
    assert!(err.to_string().contains("ascending"), "got: {err}");
}

#[test]
fn a_conversion_block_shorter_than_its_parameters_is_refused() {
    // The block declares six polynomial parameters and carries two. Reading the
    // missing four as zero would give a formula the file never stated.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "short_cc.mdf",
        Cc::Raw {
            code: 6,
            count: 6,
            params: vec![0u8; 16],
        },
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let err = match file.physical_by_name("X") {
        Ok(_) => panic!("a conversion block short of its parameters must be refused"),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidBlockSize { .. }),
        "got: {err}"
    );
}

#[test]
fn a_formula_this_build_cannot_parse_is_refused_at_parse_time() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "bad_formula.mdf",
        Cc::Formula("3 * frobnicate(X)"),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let err = match file.physical_by_name("X") {
        Ok(_) => panic!("a formula naming an unknown function must be refused"),
        Err(e) => e,
    };
    assert!(
        matches!(err, falcon_mdf::Mf4Error::InvalidConversion { .. }),
        "got: {err}"
    );
}

#[test]
fn a_text_conversion_reports_itself_as_producing_text() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = conversion_file(
        dir.path(),
        "output_kind.mdf",
        Cc::TextTable(vec![(1.0, "one")]),
    );
    let file = Mdf3File::open(&path).expect("falcon should open it");
    let (g, c, i) = index_of(&file, "X");
    let cc = file.channel_conversion(g, c, i).unwrap();
    assert_eq!(cc.output(), Mdf3ConversionOutput::Text);
    // Asking a text rule for a number is a caller error, not a silent NaN.
    assert!(cc.convert(1.0).is_err());
}
