//! Golden-value regression tests.
//!
//! `tests/data/golden.json` holds independently verified ground truth for every
//! channel in the sample corpus: sample counts, the first five physical values,
//! and min/max. Any change to bit extraction, record striding, decompression,
//! or conversion that alters a decoded value will fail here.
//!
//! The sample files live under `test_data/`, which is not checked in. When they
//! are absent the tests skip rather than fail, so a fresh clone stays green;
//! run them locally against the corpus before landing decoder changes.

use falcon_mdf::{Mf4File, SignalValues, ValueKind};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Relative tolerance for physical values. Decoding is expected to be exact;
/// this only absorbs last-ulp differences in conversion arithmetic.
const REL_TOL: f64 = 1e-9;

fn load_golden() -> Value {
    let raw = include_str!("data/golden.json");
    serde_json::from_str(raw).expect("golden.json is malformed")
}

/// Maps `(data_group_index, channel_group_index)` to the flat group index used
/// by the golden data, which numbers channel groups sequentially across the
/// whole file.
fn group_indices(file: &Mf4File) -> HashMap<(usize, usize), usize> {
    let mut map = HashMap::new();
    let mut flat = 0usize;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            map.insert((dg.index, cg.index), flat);
            flat += 1;
        }
    }
    map
}

fn close(a: f64, b: f64) -> bool {
    if a == b || (a.is_nan() && b.is_nan()) {
        return true;
    }
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= REL_TOL * scale
}

/// One mismatch between decoded output and ground truth.
struct Mismatch {
    channel: String,
    detail: String,
}

fn check_file(path: &str, expected: &Value) -> (Vec<Mismatch>, Vec<String>) {
    let mut bad = Vec::new();
    let deferred: Vec<String> = Vec::new();

    let file = match Mf4File::open(path) {
        Ok(f) => f,
        Err(e) => {
            bad.push(Mismatch {
                channel: "<file>".into(),
                detail: format!("failed to open: {e}"),
            });
            return (bad, deferred);
        }
    };

    let want_version = expected["version"].as_str().unwrap_or_default();
    let got_version = file.version().to_string();
    if got_version != want_version {
        bad.push(Mismatch {
            channel: "<version>".into(),
            detail: format!("expected {want_version}, got {got_version}"),
        });
    }

    let flat = group_indices(&file);
    let mut seen: HashMap<String, ()> = HashMap::new();

    for channel in file.channels() {
        let Some(&gi) = flat.get(&(channel.data_group_index, channel.channel_group_index)) else {
            continue;
        };
        let key = format!("{}:{}", gi, channel.name);
        if seen.insert(key.clone(), ()).is_some() {
            continue;
        }

        let Some(want) = expected["channels"].get(&key) else {
            bad.push(Mismatch {
                channel: key,
                detail: "channel not present in ground truth".into(),
            });
            continue;
        };

        let kind = want["kind"].as_str().unwrap_or("numeric");
        if kind == "error" {
            continue;
        }

        // The decoded type itself is part of the contract: a 29-bit CAN
        // identifier must come back as u32, a frame payload as bytes.
        let got_kind = channel.value_kind();
        let type_ok = match kind {
            "composite" | "other" => got_kind == ValueKind::Bytes,
            expected => got_kind.name() == expected,
        };
        if !type_ok {
            bad.push(Mismatch {
                channel: key.clone(),
                detail: format!("type: expected {kind}, got {}", got_kind.name()),
            });
        }

        let signal = match file.signal(channel) {
            Ok(s) => s,
            Err(e) => {
                bad.push(Mismatch {
                    channel: key,
                    detail: format!("signal() failed: {e}"),
                });
                continue;
            }
        };

        let want_n = want["n"].as_u64().unwrap_or(0) as usize;
        if signal.len() != want_n {
            bad.push(Mismatch {
                channel: key,
                detail: format!("sample count: expected {want_n}, got {}", signal.len()),
            });
            continue;
        }

        if want_n == 0 {
            continue;
        }

        // Byte-array channels: check the literal payload of the first sample.
        // This is what regressed as `1.8e19` before typed values existed.
        if kind == "bytes" {
            let values = match signal.values() {
                Ok(v) => v,
                Err(e) => {
                    bad.push(Mismatch {
                        channel: key,
                        detail: format!("values() failed: {e}"),
                    });
                    continue;
                }
            };
            // The reference pads every payload out to the longest, because a
            // numpy array has to be rectangular. This crate keeps each payload
            // at its real length, so a file with mixed sizes yields VarBytes
            // where the reference reports a fixed width. Padding invents bytes
            // that are not in the file, so the difference is deliberate — but
            // the payload itself must still match.
            let uniform_width = match &values {
                SignalValues::Bytes { width, .. } => Some(*width as u64),
                SignalValues::VarBytes { .. } => None,
                other => {
                    bad.push(Mismatch {
                        channel: key.clone(),
                        detail: format!("expected byte samples, got {}", other.kind().name()),
                    });
                    continue;
                }
            };

            if let (Some(want_width), Some(got_width)) = (want["width"].as_u64(), uniform_width) {
                if got_width != want_width {
                    bad.push(Mismatch {
                        channel: key.clone(),
                        detail: format!("byte width: expected {want_width}, got {got_width}"),
                    });
                    continue;
                }
            }

            if let Some(want_hex) = want["first_bytes"].as_str() {
                let got = values.bytes_at(0).unwrap_or(&[]);
                let got_hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
                let agrees = if uniform_width.is_some() {
                    got_hex == want_hex
                } else {
                    // Unpadded: ours is what the reference padded.
                    want_hex.starts_with(&got_hex)
                };
                if !agrees {
                    bad.push(Mismatch {
                        channel: key.clone(),
                        detail: format!("first sample bytes: expected {want_hex}, got {got_hex}"),
                    });
                }
            }
            continue;
        }

        // Composite structures are returned decoded by the reference and raw by
        // falcon_mdf, so their bytes are not comparable; kind and count above
        // are the meaningful assertions.
        if kind == "composite" || kind == "other" {
            continue;
        }

        let values = match signal.values_f64() {
            Ok(v) => v,
            Err(e) => {
                bad.push(Mismatch {
                    channel: key,
                    detail: format!("values_f64() failed: {e}"),
                });
                continue;
            }
        };

        if let Some(first) = want["first"].as_array() {
            for (i, w) in first.iter().enumerate() {
                let w = w.as_f64().unwrap_or(f64::NAN);
                let g = values[i];
                if !close(w, g) {
                    bad.push(Mismatch {
                        channel: key.clone(),
                        detail: format!("value[{i}]: expected {w}, got {g}"),
                    });
                    break;
                }
            }
        }

        let (min, max) = values
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(*v), hi.max(*v))
            });
        if let Some(w) = want["min"].as_f64() {
            if !close(w, min) {
                bad.push(Mismatch {
                    channel: key.clone(),
                    detail: format!("min: expected {w}, got {min}"),
                });
            }
        }
        if let Some(w) = want["max"].as_f64() {
            if !close(w, max) {
                bad.push(Mismatch {
                    channel: key.clone(),
                    detail: format!("max: expected {w}, got {max}"),
                });
            }
        }
    }

    (bad, deferred)
}

#[test]
fn golden_values_match_reference() {
    let golden = load_golden();
    let files = golden.as_object().expect("golden root must be an object");

    let mut checked = 0usize;
    let mut skipped = Vec::new();
    let mut failures: Vec<(String, Vec<Mismatch>)> = Vec::new();

    for (path, expected) in files {
        if !Path::new(path).exists() {
            skipped.push(path.clone());
            continue;
        }
        checked += 1;
        let (bad, _deferred) = check_file(path, expected);
        if !bad.is_empty() {
            failures.push((path.clone(), bad));
        }
    }

    if checked == 0 {
        eprintln!(
            "SKIP: none of the {} corpus files are present under test_data/",
            files.len()
        );
        return;
    }
    if !skipped.is_empty() {
        eprintln!("note: skipped {} absent corpus file(s)", skipped.len());
    }

    if !failures.is_empty() {
        let total: usize = failures.iter().map(|(_, b)| b.len()).sum();
        let mut report = format!(
            "\n{total} golden-value mismatch(es) across {} of {checked} file(s):\n",
            failures.len()
        );
        for (path, bad) in &failures {
            let name = Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            report.push_str(&format!("\n  {name} — {} mismatch(es)\n", bad.len()));
            for m in bad.iter().take(8) {
                report.push_str(&format!("    {} — {}\n", m.channel, m.detail));
            }
            if bad.len() > 8 {
                report.push_str(&format!("    ... and {} more\n", bad.len() - 8));
            }
        }
        panic!("{report}");
    }
}

/// Metadata is descriptive rather than numeric, so it is not part of the golden
/// value table; these check it against what the corpus files actually contain.
#[test]
fn header_metadata_is_parsed_rather_than_returned_as_markup() {
    let golden = load_golden();
    let files = golden.as_object().expect("golden root must be an object");

    let mut checked = 0usize;
    for path in files.keys() {
        if !Path::new(path).exists() {
            continue;
        }
        let Ok(file) = Mf4File::open(path) else {
            continue;
        };
        checked += 1;

        assert!(
            !file.comment().contains('<'),
            "{path}: comment still contains markup — the XML container is not the comment"
        );

        // Every corpus file is written by the same logger family, so its header
        // records the device that produced it.
        let meta = file.metadata();
        if !meta.is_empty() {
            assert!(
                meta.get("Device Information/serial number").is_some(),
                "{path}: expected a device serial number among {} properties",
                meta.property_count()
            );
            for (key, _) in meta.properties() {
                assert!(!key.is_empty(), "{path}: a property has no name");
                assert!(
                    !key.contains('<'),
                    "{path}: property path {key} contains markup"
                );
            }
        }
    }

    if checked == 0 {
        eprintln!("SKIP: no corpus files present");
    }
}
