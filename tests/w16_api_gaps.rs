//! W16 — Signal operator algebra and channel search.
//!
//! `scramble` is deliberately left out of this worktree; see the report.

use falcon_mdf::{ChannelLocation, Mf4File, Mf4Writer, SearchMode, SignalSeries, SignalValues};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

type AsammdfOpResult = (Vec<f64>, Vec<f64>, Option<Vec<bool>>);

/// Runs an asammdf Signal operation and returns (timestamps, values, validity).
fn asammdf_signal_op(
    left_ts: &[f64],
    left_vals: &[f64],
    right_ts: &[f64],
    right_vals: &[f64],
    op: &str,
) -> Option<AsammdfOpResult> {
    let python = venv_python()?;
    if !asammdf_available(&python) {
        return None;
    }

    let left_ts: Vec<f64> = left_ts.to_vec();
    let left_vals: Vec<f64> = left_vals.to_vec();
    let right_ts: Vec<f64> = right_ts.to_vec();
    let right_vals: Vec<f64> = right_vals.to_vec();

    let input = serde_json::json!({
        "left_ts": left_ts,
        "left_vals": left_vals,
        "right_ts": right_ts,
        "right_vals": right_vals,
        "op": op,
    });

    let mut child = Command::new(python)
        .arg("-c")
        .arg(
            r#"
import json
import struct
import numpy as np
from asammdf import Signal

inp = json.load(open(0))
op = inp["op"]

s1 = Signal(np.array(inp["left_vals"], dtype=np.float64),
            np.array(inp["left_ts"], dtype=np.float64),
            name="left", unit="u")
s2 = Signal(np.array(inp["right_vals"], dtype=np.float64),
            np.array(inp["right_ts"], dtype=np.float64),
            name="right", unit="u")

method = getattr(s1, op)
r = method(s2)

ts = r.timestamps.tolist()
# comparisons return bool; promote to float so the bit-pattern transport works
vals = r.samples.astype(np.float64).tolist()
bits = [int.from_bytes(struct.pack("<d", v), "little") for v in vals]
validity = None
if r.invalidation_bits is not None:
    validity = r.invalidation_bits.tolist()

print(json.dumps({"timestamps": ts, "values": bits, "validity": validity}))
"#,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python");

    {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        stdin
            .write_all(input.to_string().as_bytes())
            .expect("write to python stdin");
    }

    let out = child.wait_with_output().expect("wait for python");

    assert!(
        out.status.success(),
        "asammdf op failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().last().expect("python produced no output");
    let parsed: serde_json::Value =
        serde_json::from_str(line).expect("python output should be JSON");

    let ts = parsed["timestamps"]
        .as_array()
        .expect("timestamps array")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let vals = parsed["values"]
        .as_array()
        .expect("values array")
        .iter()
        .map(|v| f64::from_bits(v.as_u64().unwrap()))
        .collect();
    let validity = parsed["validity"].as_array().map(|arr| {
        arr.iter()
            .map(|v| v.as_bool().expect("validity entry should be bool"))
            .collect()
    });
    Some((ts, vals, validity))
}

/// Builds a `SignalSeries` from f64 samples.
fn series(name: &str, unit: &str, ts: &[f64], vals: &[f64]) -> SignalSeries {
    SignalSeries::from_samples(
        name,
        unit,
        ts.to_vec(),
        SignalValues::F64(vals.to_vec()),
        None,
    )
    .unwrap()
}

#[test]
fn signal_addition_matches_asammdf_on_mixed_timebases() {
    let a = series("a", "u", &[0.0, 1.0, 2.0, 3.0], &[10.0, 20.0, 30.0, 40.0]);
    let b = series("b", "u", &[0.5, 1.5, 2.5], &[5.0, 15.0, 25.0]);

    let got = &a + &b;
    let Some((want_ts, want_vals, _)) = asammdf_signal_op(
        &[0.0, 1.0, 2.0, 3.0],
        &[10.0, 20.0, 30.0, 40.0],
        &[0.5, 1.5, 2.5],
        &[5.0, 15.0, 25.0],
        "__add__",
    ) else {
        eprintln!("skipping: asammdf not available");
        return;
    };

    assert_eq!(got.timestamps(), want_ts.as_slice());
    assert_eq!(got.values_f64(), want_vals);
}

#[test]
fn signal_subtraction_matches_asammdf_on_mixed_timebases() {
    let a = series("a", "u", &[0.0, 1.0, 2.0, 3.0], &[10.0, 20.0, 30.0, 40.0]);
    let b = series("b", "u", &[0.5, 1.5, 2.5], &[5.0, 15.0, 25.0]);

    let got = &a - &b;
    let Some((want_ts, want_vals, _)) = asammdf_signal_op(
        &[0.0, 1.0, 2.0, 3.0],
        &[10.0, 20.0, 30.0, 40.0],
        &[0.5, 1.5, 2.5],
        &[5.0, 15.0, 25.0],
        "__sub__",
    ) else {
        eprintln!("skipping: asammdf not available");
        return;
    };

    assert_eq!(got.timestamps(), want_ts.as_slice());
    assert_eq!(got.values_f64(), want_vals);
}

#[test]
fn signal_multiplication_matches_asammdf_on_mixed_timebases() {
    let a = series("a", "u", &[0.0, 1.0, 2.0, 3.0], &[10.0, 20.0, 30.0, 40.0]);
    let b = series("b", "u", &[0.5, 1.5, 2.5], &[2.0, 3.0, 4.0]);

    let got = &a * &b;
    let Some((want_ts, want_vals, _)) = asammdf_signal_op(
        &[0.0, 1.0, 2.0, 3.0],
        &[10.0, 20.0, 30.0, 40.0],
        &[0.5, 1.5, 2.5],
        &[2.0, 3.0, 4.0],
        "__mul__",
    ) else {
        eprintln!("skipping: asammdf not available");
        return;
    };

    assert_eq!(got.timestamps(), want_ts.as_slice());
    assert_eq!(got.values_f64(), want_vals);
}

#[test]
fn signal_division_matches_asammdf_on_mixed_timebases() {
    let a = series("a", "u", &[0.0, 1.0, 2.0, 3.0], &[10.0, 20.0, 30.0, 40.0]);
    let b = series("b", "u", &[0.5, 1.5, 2.5], &[2.0, 4.0, 5.0]);

    let got = &a / &b;
    let Some((want_ts, want_vals, _)) = asammdf_signal_op(
        &[0.0, 1.0, 2.0, 3.0],
        &[10.0, 20.0, 30.0, 40.0],
        &[0.5, 1.5, 2.5],
        &[2.0, 4.0, 5.0],
        "__truediv__",
    ) else {
        eprintln!("skipping: asammdf not available");
        return;
    };

    assert_eq!(got.timestamps(), want_ts.as_slice());
    assert_eq!(got.values_f64(), want_vals);
}

#[test]
fn signal_comparisons_match_asammdf() {
    let a = series("a", "u", &[0.0, 1.0, 2.0, 3.0], &[10.0, 20.0, 30.0, 40.0]);
    let b = series("b", "u", &[0.5, 1.5, 2.5], &[15.0, 20.0, 35.0]);

    for (method, op) in [
        (
            SignalSeries::lt
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__lt__",
        ),
        (
            SignalSeries::le
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__le__",
        ),
        (
            SignalSeries::gt
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__gt__",
        ),
        (
            SignalSeries::ge
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__ge__",
        ),
        (
            SignalSeries::eq_samples
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__eq__",
        ),
        (
            SignalSeries::ne_samples
                as fn(&SignalSeries, &SignalSeries) -> falcon_mdf::Result<SignalSeries>,
            "__ne__",
        ),
    ] {
        let got = method(&a, &b).expect("comparison should succeed");
        let Some((want_ts, want_vals, _)) = asammdf_signal_op(
            &[0.0, 1.0, 2.0, 3.0],
            &[10.0, 20.0, 30.0, 40.0],
            &[0.5, 1.5, 2.5],
            &[15.0, 20.0, 35.0],
            op,
        ) else {
            eprintln!("skipping: asammdf not available");
            return;
        };
        assert_eq!(got.timestamps(), want_ts.as_slice(), "timestamps for {op}");
        assert_eq!(got.values_f64(), want_vals, "values for {op}");
    }
}

#[test]
fn signal_scalar_operators_work_without_resampling() {
    let a = series("a", "u", &[0.0, 1.0, 2.0], &[10.0, 20.0, 30.0]);

    let plus = &a + 5.0;
    assert_eq!(plus.timestamps(), a.timestamps());
    assert_eq!(plus.values_f64(), vec![15.0, 25.0, 35.0]);

    let minus = &a - 3.0;
    assert_eq!(minus.values_f64(), vec![7.0, 17.0, 27.0]);

    let times = &a * 2.0;
    assert_eq!(times.values_f64(), vec![20.0, 40.0, 60.0]);

    let div = &a / 2.0;
    assert_eq!(div.values_f64(), vec![5.0, 10.0, 15.0]);

    let neg = -&a;
    assert_eq!(neg.values_f64(), vec![-10.0, -20.0, -30.0]);

    let rev_sub = a.sub_from_scalar(100.0).unwrap();
    assert_eq!(rev_sub.values_f64(), vec![90.0, 80.0, 70.0]);

    let rev_div = a.div_by_scalar(1.0).unwrap();
    assert_eq!(rev_div.values_f64(), vec![0.1, 0.05, 1.0 / 30.0]);
}

#[test]
fn signal_invalidity_propagates_with_logical_or() {
    let a = SignalSeries::from_samples(
        "a",
        "u",
        vec![0.0, 1.0, 2.0],
        SignalValues::F64(vec![10.0, 20.0, 30.0]),
        Some(vec![true, false, true]),
    )
    .unwrap();
    let b = SignalSeries::from_samples(
        "b",
        "u",
        vec![0.0, 1.0, 2.0],
        SignalValues::F64(vec![1.0, 2.0, 3.0]),
        Some(vec![false, false, true]),
    )
    .unwrap();

    let sum = a.add(&b).unwrap();
    assert_eq!(sum.validity(), Some(&[true, false, true][..]));
}

#[test]
fn channel_locations_returns_every_match() {
    let mut writer = Mf4Writer::new();
    let g1 = writer.add_group(&[0.0, 1.0]).unwrap();
    g1.add_channel("Speed", "km/h", &[10.0, 20.0]).unwrap();

    let g2 = writer.add_group(&[0.0, 1.0]).unwrap();
    g2.add_channel("Speed", "mph", &[6.0, 12.0]).unwrap();
    g2.add_channel("RPM", "rpm", &[1000.0, 2000.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let locs = file.find_channel_locations("Speed");
    assert_eq!(locs.len(), 2);
    assert_eq!(locs[0], ChannelLocation::new(0, 0, 1));
    assert_eq!(locs[1], ChannelLocation::new(1, 0, 1));

    let rpm = file.find_channel_locations("RPM");
    assert_eq!(rpm, vec![ChannelLocation::new(1, 0, 2)]);

    let none = file.find_channel_locations("Missing");
    assert!(none.is_empty());
}

#[test]
fn channel_search_finds_substrings_and_wildcards() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[0.0]).unwrap();
    g.add_channel("EngineSpeed", "rpm", &[1000.0]).unwrap();
    g.add_channel("EngineTemp", "C", &[90.0]).unwrap();
    g.add_channel("VehicleSpeed", "km/h", &[50.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let plain: Vec<String> = file.search_channels("Speed", SearchMode::Plain);
    assert_eq!(plain, vec!["EngineSpeed", "VehicleSpeed"]);

    let case: Vec<String> = file.search_channels("speed", SearchMode::CaseInsensitive);
    assert_eq!(case, vec!["EngineSpeed", "VehicleSpeed"]);

    let wildcard: Vec<String> = file.search_channels("Engine*", SearchMode::Wildcard);
    assert_eq!(wildcard, vec!["EngineSpeed", "EngineTemp"]);

    let none = file.search_channels("Pressure", SearchMode::Plain);
    assert!(none.is_empty());
}
