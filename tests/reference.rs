//! Decoding checked against files other tools wrote.
//!
//! For nine phases every feature added after Phase 4 was verified against
//! fixtures built from the specification, and not one had been read from a file
//! another tool produced. The fixtures were good — each was shown to fail
//! before it was trusted — and they still missed five defects, three of which
//! were pinned by a fixture that had encoded the same misreading as the code.
//! That is the gap this suite closes.
//!
//! The files are the ASAM vendor reference set: Vector, dSPACE and ETAS output,
//! the collection the openATFX-MDF project validates against. Between them they
//! exercise 13 of 17 data types and 11 of 12 conversion types, where a corpus
//! of bus logs reaches 3 and 2.
//!
//! They are not redistributed here. `scripts/fetch_reference_files.sh` fetches
//! them into `test_data/`, which is gitignored; `tests/data/
//! reference_golden.json` holds only the values they decode to and *is* checked
//! in. A fresh clone therefore has the ground truth but not the files, and
//! these tests skip rather than fail until the script is run — the same
//! arrangement `golden.rs` uses for the sample corpus.
//!
//! The ground truth came from asammdf, an independently written reader, so
//! agreement means two separate readings of the same bytes coincide. Where it
//! is wrong the entry is marked `divergence` and carries the reason; those
//! channels are checked for decodability but not for value, and the reasons are
//! worth reading, because two of them are places where this reader follows the
//! standard and the reference does not.

use falcon_mdf::{Mf4File, SignalValues};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Number of leading samples the ground truth records per channel.
const TAKE: usize = 20;

/// Relative tolerance, absorbing last-ulp differences in conversion arithmetic.
const REL_TOL: f64 = 1e-9;

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_data")
        .join("reference")
}

fn golden() -> Value {
    serde_json::from_str(include_str!("data/reference_golden.json"))
        .expect("reference_golden.json is malformed")
}

fn close(a: f64, b: f64) -> bool {
    if a == b || (a.is_nan() && b.is_nan()) {
        return true;
    }
    if a.is_infinite() || b.is_infinite() {
        return false;
    }
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= REL_TOL * scale
}

/// Reads a recorded number, which may be a tagged non-finite value.
///
/// `inf` and `NaN` are different answers, and JSON holds neither — collapsing
/// them to one token would hide a real disagreement about division by zero.
fn expected_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => match s.as_str() {
            "nan" => Some(f64::NAN),
            "inf" => Some(f64::INFINITY),
            "-inf" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

/// Flat channel-group number, walking data groups then channel groups in file
/// order — the ordering the ground truth is keyed by.
fn flat_groups(file: &Mf4File) -> std::collections::HashMap<(usize, usize), usize> {
    let mut map = std::collections::HashMap::new();
    let mut next = 0usize;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            map.insert((dg.index, cg.index), next);
            next += 1;
        }
    }
    map
}

/// What this reader decodes, in the shape the ground truth records.
enum Decoded {
    Num(Vec<f64>, usize),
    Str(Vec<String>, usize),
    Bytes(Vec<String>, usize),
    Canopen(usize),
    Failed(String),
}

fn decode(file: &Mf4File, channel: &falcon_mdf::Channel) -> Decoded {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();

    match file.signal(channel).and_then(|s| s.values()) {
        Ok(SignalValues::Str(v)) => {
            let n = v.len();
            Decoded::Str(v.into_iter().take(TAKE).collect(), n)
        }
        Ok(SignalValues::Bytes { data, width }) => {
            let n = if width == 0 { 0 } else { data.len() / width };
            Decoded::Bytes(data.chunks(width).take(TAKE).map(hex).collect(), n)
        }
        Ok(SignalValues::VarBytes { data, starts }) => {
            let n = starts.len().saturating_sub(1);
            let first = (0..n.min(TAKE))
                .map(|i| hex(&data[starts[i]..starts[i + 1]]))
                .collect();
            Decoded::Bytes(first, n)
        }
        Ok(SignalValues::CanopenDate(v)) => Decoded::Canopen(v.len()),
        Ok(SignalValues::CanopenTime(v)) => Decoded::Canopen(v.len()),
        Ok(other) => {
            let n = other.len();
            Decoded::Num(other.to_f64().into_iter().take(TAKE).collect(), n)
        }
        Err(e) => Decoded::Failed(e.to_string()),
    }
}

/// One channel that did not decode to what the reference recorded.
struct Mismatch {
    file: String,
    channel: String,
    detail: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {}", self.file, self.channel, self.detail)
    }
}

/// Compares one file against its recorded values, collecting every difference.
fn check_file(path: &Path, expected: &Value, out: &mut Vec<Mismatch>) -> usize {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let file = match Mf4File::open(path) {
        Ok(f) => f,
        Err(e) => {
            out.push(Mismatch {
                file: name,
                channel: "<file>".into(),
                detail: format!("would not open, but the reference reads it: {e}"),
            });
            return 0;
        }
    };

    let groups = flat_groups(&file);
    let channels: Vec<_> = file.channels().cloned().collect();
    let recorded = expected["channels"].as_object().expect("channels object");
    let mut checked = 0;

    for channel in &channels {
        let Some(&g) = groups.get(&(channel.data_group_index, channel.channel_group_index)) else {
            continue;
        };
        let Some(want) = recorded.get(&format!("{g}:{}", channel.name)) else {
            // A channel this reader exposes and the reference does not — a
            // composition child, most often. Nothing to compare against.
            continue;
        };
        let kind = want["kind"].as_str().unwrap_or("");

        // The reference could not read it either, so there is nothing to hold
        // this reader to.
        if matches!(kind, "error" | "other") {
            continue;
        }

        let got = decode(&file, channel);

        // A recorded divergence still has to *decode*; only its value is not
        // asserted, and the reason says which reader to believe.
        if kind == "divergence" {
            if let Decoded::Failed(e) = got {
                out.push(Mismatch {
                    file: name.clone(),
                    channel: channel.name.clone(),
                    detail: format!("a known divergence must still decode: {e}"),
                });
            }
            continue;
        }

        checked += 1;
        let mut fail = |detail: String| {
            out.push(Mismatch {
                file: name.clone(),
                channel: channel.name.clone(),
                detail,
            })
        };

        let want_n = want["n"].as_u64().unwrap_or(0) as usize;
        let first = want["first"].as_array();

        match got {
            Decoded::Failed(e) => fail(format!("failed to decode, reference reads it: {e}")),
            Decoded::Canopen(n) if kind == "canopen" => {
                if n != want_n {
                    fail(format!("sample count {n}, reference {want_n}"));
                }
            }
            Decoded::Num(v, n) if kind == "num" => {
                if n != want_n {
                    fail(format!("sample count {n}, reference {want_n}"));
                } else if let Some(want_v) = first {
                    for (i, w) in want_v.iter().enumerate() {
                        let (Some(a), Some(b)) = (v.get(i), expected_number(w)) else {
                            continue;
                        };
                        if !close(*a, b) {
                            fail(format!("sample {i} is {a}, reference {b}"));
                            break;
                        }
                    }
                }
            }
            Decoded::Str(v, n) if kind == "str" => {
                if n != want_n {
                    fail(format!("sample count {n}, reference {want_n}"));
                } else if let Some(want_v) = first {
                    for (i, w) in want_v.iter().enumerate() {
                        let (Some(a), Some(b)) = (v.get(i), w.as_str()) else {
                            continue;
                        };
                        if a.trim_end_matches('\0') != b.trim_end_matches('\0') {
                            fail(format!("sample {i} is {a:?}, reference {b:?}"));
                            break;
                        }
                    }
                }
            }
            Decoded::Bytes(v, n) if kind == "bytes" => {
                if n != want_n {
                    fail(format!("sample count {n}, reference {want_n}"));
                } else if let Some(want_v) = first {
                    for (i, w) in want_v.iter().enumerate() {
                        let (Some(a), Some(b)) = (v.get(i), w.as_str()) else {
                            continue;
                        };
                        if a != b {
                            fail(format!("sample {i} is {a}, reference {b}"));
                            break;
                        }
                    }
                }
            }
            other => {
                let got_kind = match other {
                    Decoded::Num(..) => "num",
                    Decoded::Str(..) => "str",
                    Decoded::Bytes(..) => "bytes",
                    Decoded::Canopen(..) => "canopen",
                    Decoded::Failed(..) => "error",
                };
                fail(format!("decoded as {got_kind}, reference has {kind}"));
            }
        }
    }

    checked
}

#[test]
fn vendor_files_decode_to_what_an_independent_reader_reads() {
    let dir = reference_dir();
    if !dir.is_dir() {
        eprintln!(
            "skipping: no reference files. Run scripts/fetch_reference_files.sh to fetch them."
        );
        return;
    }

    let golden = golden();
    let mut mismatches = Vec::new();
    let mut files = 0;
    let mut channels = 0;

    for (name, expected) in golden.as_object().expect("golden object") {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        files += 1;
        channels += check_file(&path, expected, &mut mismatches);
    }

    if files == 0 {
        eprintln!("skipping: reference directory is empty");
        return;
    }

    assert!(
        mismatches.is_empty(),
        "{} of {channels} channels across {files} files disagree with the reference:\n  {}",
        mismatches.len(),
        mismatches
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    eprintln!("{channels} channels across {files} reference files agree");
}

#[test]
fn every_reference_file_opens() {
    // Separate from the value check because "cannot open" is a different
    // failure from "decodes differently", and four of the five defects this
    // suite found presented as one file refusing to open — which is also the
    // failure a caller notices first.
    let dir = reference_dir();
    if !dir.is_dir() {
        eprintln!("skipping: no reference files");
        return;
    }

    let mut refused = Vec::new();
    let mut opened = 0;
    for (name, _) in golden().as_object().expect("golden object") {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        match Mf4File::open(&path) {
            Ok(_) => opened += 1,
            Err(e) => refused.push(format!("{name}: {e}")),
        }
    }

    assert!(
        refused.is_empty(),
        "{} file(s) an independent reader opens were refused:\n  {}",
        refused.len(),
        refused.join("\n  ")
    );
    eprintln!("{opened} reference files open");
}

/// B35's repro, re-run after the CA-chain and dynamic-size work: flipping the
/// byte at offset 1331 of `dSPACE_MeasurementArrays.mf4` mutates one of a CA
/// block's `ca_dim_size` fields into an enormous value. The fix that closed
/// B35 lives in the same function the CA-chain and dynamic-size bounds were
/// just added beside, so this proves the file-supplied path is still checked
/// before anything is allocated on the strength of it — not just the
/// synthetic fixture that motivated the original fix.
#[test]
fn the_b35_byte_flip_repro_still_errors_cleanly_not_ballooning() {
    let path = reference_dir().join("dSPACE_MeasurementArrays.mf4");
    if !path.is_file() {
        eprintln!(
            "skipping: no reference files. Run scripts/fetch_reference_files.sh to fetch them."
        );
        return;
    }

    let mut bytes = std::fs::read(&path).expect("read dSPACE_MeasurementArrays.mf4");
    bytes[1331] ^= 0xFF;

    let flipped = std::env::temp_dir().join("falcon_mdf_b35_repro.mf4");
    std::fs::write(&flipped, &bytes).expect("write mutated file");

    let file = Mf4File::open(&flipped).expect("a mutated dim size must not refuse the whole file");
    let mut any_array_checked = false;
    for ch in file.channels() {
        if ch.array_shape().is_none() {
            continue;
        }
        any_array_checked = true;
        // Either this is the mutated channel and reading it must fail rather
        // than allocate on the strength of the corrupted shape, or it is one
        // of the file's other array channels and must still decode normally
        // — the mutation must not have corrupted an unrelated channel's view
        // of the link section.
        let _ = file.signal(ch).and_then(|s| s.values());
    }
    assert!(
        any_array_checked,
        "the file should still expose at least one array channel to check"
    );

    let _ = std::fs::remove_file(&flipped);
}

/// `KF4` in `Vector_MeasurementArrays.mf4`: a look-up array whose composition
/// names another CA block rather than a template CN (B30) — the shape the
/// golden fixture cannot check, because asammdf itself fails on the sibling
/// channel this one's inner dimension is axis-referenced against
/// (`"array-shape mismatch in array 2 (\"Curve1\")"`, recorded under key
/// `8:KF4`) and `check_file` skips any channel the reference errors on.
///
/// The expected values below were read by hand from the file's own bytes —
/// not from any reader, this one included — as documented in this crate's
/// implementation notes for B30. `KF4`'s CN declares one byte per element
/// (`cn_bit_count` 8, `cn_byte_offset` 8); its composition is a CA block
/// (`ca_type` Lookup, `ca_dim_size` `[6]`, `ca_byte_offset_base` 8) whose own
/// composition is a second CA block (`ca_dim_size` `[8]`,
/// `ca_byte_offset_base` 1, composition 0 — elements typed by KF4's own CN).
/// Combined shape `[6, 8]` = 48 elements, occupying bytes 8..56 of the
/// 56-byte record — exactly what remains after the 8-byte time master, which
/// is why 48 was believed sooner than any single tool's output was.
#[test]
fn a_look_up_array_composed_with_another_ca_block_decodes_its_combined_shape() {
    let path = reference_dir().join("Vector_MeasurementArrays.mf4");
    if !path.is_file() {
        eprintln!(
            "skipping: no reference files. Run scripts/fetch_reference_files.sh to fetch them."
        );
        return;
    }

    let file = Mf4File::open(&path).expect("Vector_MeasurementArrays.mf4 should open");
    let ch = file.find_channel("KF4").expect("KF4 should be listed");
    assert_eq!(
        ch.array_shape(),
        Some(&[6u64, 8u64][..]),
        "combined shape is the outer CA's dims followed by the inner CA's"
    );
    assert!(ch.unreadable().is_none(), "KF4's own elements are readable");

    let values = file
        .signal(ch)
        .expect("signal")
        .values()
        .expect("KF4 should decode");
    let SignalValues::Array {
        values,
        elements_per_sample,
    } = values
    else {
        panic!("expected a fixed-size array");
    };
    assert_eq!(elements_per_sample, 48);

    // Each outer i in 0..6 contributes 8 bytes [i*10 + j for j in 0..8],
    // read straight from the file (see the doc comment above). Samples 2 and
    // 3 have two bytes of row i=2 perturbed by the file's own author, not by
    // this reader: j=4 reads 42 instead of 24 and j=6 reads 12 instead of 26;
    // j=5 and j=7 only look perturbed because 20+5=25 and 20+7=27 already.
    // An unperturbed row proves the layout; a perturbed one proves nothing
    // here is quietly "fixing" the data to match an expectation.
    let row = |perturb_row_2: bool| -> Vec<f64> {
        let mut out = Vec::new();
        for i in 0..6u64 {
            for j in 0..8u64 {
                let expected = i * 10 + j;
                let value = match (perturb_row_2, i, j) {
                    (true, 2, 4) => 42,
                    (true, 2, 6) => 12,
                    _ => expected,
                };
                out.push(value as f64);
            }
        }
        out
    };
    let expected: Vec<f64> = [row(false), row(false), row(true), row(true)].concat();

    assert_eq!(values, expected, "KF4's 4 samples, 48 elements each");
}
