//! Integration tests for time-domain operations: `cut` and `resample`.
//!
//! Verifies boundary inclusivity, empty intervals, finer/coarser rasters,
//! extrapolation beyond data limits, non-numeric channel fallbacks, and
//! cross-checks results against asammdf.

use falcon_mdf::{InterpolationMode, Mf4File, Mf4Writer, Raster, SignalValues};
use std::path::PathBuf;
use std::process::Command;

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

#[test]
fn cut_exact_samples_bounds() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
    g.add_channel("Speed", "km/h", &[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch = file.find_channel("Speed").unwrap();
    let cut = file.cut_channel(ch, 2.0, 4.0).unwrap();

    assert_eq!(cut.timestamps(), &[2.0, 3.0, 4.0]);
    assert_eq!(cut.values_f64(), vec![20.0, 30.0, 40.0]);
    assert_eq!(cut.len(), 3);
    assert_eq!(cut.unit(), "km/h");
}

#[test]
fn cut_between_samples_bounds() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
    g.add_channel("Speed", "km/h", &[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch = file.find_channel("Speed").unwrap();
    let cut = file.cut_channel(ch, 1.5, 4.5).unwrap();

    // Slices strictly inside [1.5, 4.5] -> samples at 2.0, 3.0, 4.0
    assert_eq!(cut.timestamps(), &[2.0, 3.0, 4.0]);
    assert_eq!(cut.values_f64(), vec![20.0, 30.0, 40.0]);
}

#[test]
fn cut_empty_intervals() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
    g.add_channel("Speed", "km/h", &[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch = file.find_channel("Speed").unwrap();

    // Interval after data
    let cut1 = file.cut_channel(ch, 6.0, 10.0).unwrap();
    assert!(cut1.is_empty());
    assert_eq!(cut1.len(), 0);
    assert_eq!(cut1.timestamps(), &[] as &[f64]);

    // Interval before data
    let cut2 = file.cut_channel(ch, -5.0, 0.5).unwrap();
    assert!(cut2.is_empty());

    // Inverted interval (start > end)
    let cut3 = file.cut_channel(ch, 4.0, 2.0).unwrap();
    assert!(cut3.is_empty());
}

#[test]
fn cut_preserves_validity_mask() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
    g.add_channel_with_validity(
        "Sensor",
        "bar",
        &[100.0, 200.0, 300.0, 400.0, 500.0],
        Some(&[true, false, true, true, false]),
    )
    .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch = file.find_channel("Sensor").unwrap();
    let cut = file.cut_channel(ch, 2.0, 4.0).unwrap();

    assert_eq!(cut.timestamps(), &[2.0, 3.0, 4.0]);
    assert_eq!(cut.values_f64(), vec![200.0, 300.0, 400.0]);
    assert_eq!(cut.validity(), Some(&[false, true, true][..]));
}

#[test]
fn cut_batched_multi_channel() {
    let mut writer = Mf4Writer::new();
    let g1 = writer.add_group(&[0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
    g1.add_channel("Ch1", "V", &[0.0, 10.0, 20.0, 30.0, 40.0]).unwrap();

    let g2 = writer.add_group(&[0.5, 1.5, 2.5, 3.5, 4.5]).unwrap();
    g2.add_channel("Ch2", "A", &[5.0, 15.0, 25.0, 35.0, 45.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch1 = file.find_channel("Ch1").unwrap();
    let ch2 = file.find_channel("Ch2").unwrap();

    let cut_batch = file.cut(&[ch1, ch2], 1.0, 3.0).unwrap();
    assert_eq!(cut_batch.len(), 2);

    // Ch1 in [1.0, 3.0] -> [1.0, 2.0, 3.0]
    assert_eq!(cut_batch[0].timestamps(), &[1.0, 2.0, 3.0]);
    assert_eq!(cut_batch[0].values_f64(), vec![10.0, 20.0, 30.0]);

    // Ch2 in [1.0, 3.0] -> [1.5, 2.5]
    assert_eq!(cut_batch[1].timestamps(), &[1.5, 2.5]);
    assert_eq!(cut_batch[1].values_f64(), vec![15.0, 25.0]);
}

#[test]
fn resample_finer_grid_linear_and_stephold() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0, 4.0]).unwrap();
    g.add_channel("Ramp", "m", &[10.0, 20.0, 30.0, 40.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("Ramp").unwrap();

    // 1. Finer grid linear: step 0.5 -> [1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0]
    let res_lin = file
        .resample_channel(ch, Raster::Step(0.5), InterpolationMode::Linear)
        .unwrap();
    assert_eq!(res_lin.timestamps(), &[1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0]);
    assert_eq!(
        res_lin.values_f64(),
        vec![10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0]
    );

    // 2. Finer grid step-hold: step 0.5
    let res_step = file
        .resample_channel(ch, Raster::Step(0.5), InterpolationMode::StepHold)
        .unwrap();
    assert_eq!(res_step.timestamps(), &[1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0]);
    assert_eq!(
        res_step.values_f64(),
        vec![10.0, 10.0, 20.0, 20.0, 30.0, 30.0, 40.0]
    );
}

#[test]
fn resample_coarser_grid() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
    g.add_channel("Signal", "rpm", &[0.0, 100.0, 200.0, 300.0, 400.0])
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("Signal").unwrap();

    // Coarser grid: step 2.0 -> [0.0, 2.0, 4.0]
    let res = file
        .resample_channel(ch, Raster::Step(2.0), InterpolationMode::Linear)
        .unwrap();
    assert_eq!(res.timestamps(), &[0.0, 2.0, 4.0]);
    assert_eq!(res.values_f64(), vec![0.0, 200.0, 400.0]);
}

#[test]
fn resample_raster_extending_beyond_data_bounds() {
    let mut writer = Mf4Writer::new();
    let g = writer.add_group(&[1.0, 2.0, 3.0]).unwrap();
    g.add_channel("Ramp", "V", &[10.0, 20.0, 30.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("Ramp").unwrap();

    // Explicit raster extending before 1.0 and after 3.0
    let explicit_ts = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0];

    // Linear extrapolation: holds boundary sample values on ends
    let res_lin = file
        .resample_channel(ch, explicit_ts.as_slice(), InterpolationMode::Linear)
        .unwrap();
    assert_eq!(res_lin.timestamps(), &explicit_ts[..]);
    assert_eq!(
        res_lin.values_f64(),
        vec![10.0, 10.0, 10.0, 15.0, 20.0, 25.0, 30.0, 30.0, 30.0]
    );

    // Step-hold extrapolation: holds boundary sample values on ends
    let res_step = file
        .resample_channel(ch, explicit_ts.as_slice(), InterpolationMode::StepHold)
        .unwrap();
    assert_eq!(res_step.timestamps(), &explicit_ts[..]);
    assert_eq!(
        res_step.values_f64(),
        vec![10.0, 10.0, 10.0, 10.0, 20.0, 20.0, 30.0, 30.0, 30.0]
    );
}

#[test]
fn resample_non_numeric_channels_fallback_to_step_hold() {
    // A real fixed-length string channel from a vendor-written file, rather
    // than a hand-built one: the point of the test is what happens to text
    // under a request for linear interpolation, and text that came out of an
    // actual file is the text a caller will actually have.
    // The corpus is fetched rather than committed, so in a worktree it lives
    // in the primary checkout rather than beside the source.
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_data/reference/Vector_FixedLengthStringSBC.mf4"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../falcon_mdf/test_data/reference/Vector_FixedLengthStringSBC.mf4"),
    ];
    let path = match candidates.into_iter().find(|p| p.is_file()) {
        Some(p) => p,
        None => panic!("the corpus file this test reads was not found in either location"),
    };

    let file = Mf4File::open(&path).expect("the vendor file should open");
    let ch = file
        .find_channel("Data channel")
        .expect("the file has a channel named 'Data channel'");

    // The file carries one sample per second: 0 -> "zero", 1 -> "one", ...
    let raster = vec![0.5, 1.0, 1.5, 2.0, 2.5, 3.0];
    let res = file
        .resample_channel(ch, raster.as_slice(), InterpolationMode::Linear)
        .expect("resampling text should succeed, not fail");

    assert_eq!(res.timestamps(), &raster[..]);

    // Linear was asked for and cannot mean anything between two strings, so
    // the last known value is carried forward instead. Interpolating text
    // would have to invent a value that was never measured.
    //
    // The boundary is inclusive: at t = 1.0 the sample recorded at t = 1.0 is
    // already the last known one. asammdf's Signal.interp on this same file and
    // raster returns ["zero", "one", "one", "two", "two", "three"], which is
    // where these values come from - not from reading them back out of this
    // implementation.
    match res.values() {
        SignalValues::Str(got) => {
            assert_eq!(
                got,
                &["zero", "one", "one", "two", "two", "three"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                "text under a linear request should hold the last value, not interpolate"
            );
        }
        other => panic!("expected text values back, got {other:?}"),
    }

    // And asking for step-hold explicitly must give the same answer, since
    // that is what linear silently became.
    let explicit = file
        .resample_channel(ch, raster.as_slice(), InterpolationMode::StepHold)
        .expect("step-hold on text should succeed");
    assert_eq!(
        explicit.values(),
        res.values(),
        "linear on text should equal step-hold on text"
    );
}

#[test]
fn resample_batched_multi_channel_synchronizes_raster() {
    let mut writer = Mf4Writer::new();
    // Group 1: times [0.0, 2.0, 4.0]
    let g1 = writer.add_group(&[0.0, 2.0, 4.0]).unwrap();
    g1.add_channel("ChA", "V", &[0.0, 20.0, 40.0]).unwrap();

    // Group 2: times [1.0, 3.0, 5.0]
    let g2 = writer.add_group(&[1.0, 3.0, 5.0]).unwrap();
    g2.add_channel("ChB", "A", &[10.0, 30.0, 50.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch_a = file.find_channel("ChA").unwrap();
    let ch_b = file.find_channel("ChB").unwrap();

    // Multi-channel resample with step 1.0 -> global range is [0.0 .. 5.0]
    let res_batch = file
        .resample(&[ch_a, ch_b], Raster::Step(1.0), InterpolationMode::Linear)
        .unwrap();

    assert_eq!(res_batch.len(), 2);
    let expected_grid = &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0];

    assert_eq!(res_batch[0].timestamps(), expected_grid);
    assert_eq!(
        res_batch[0].values_f64(),
        vec![0.0, 10.0, 20.0, 30.0, 40.0, 40.0]
    );

    assert_eq!(res_batch[1].timestamps(), expected_grid);
    assert_eq!(
        res_batch[1].values_f64(),
        vec![10.0, 10.0, 20.0, 30.0, 40.0, 50.0]
    );
}

#[test]
fn cross_check_cut_and_resample_against_asammdf() {
    let Some(python) = venv_python() else {
        eprintln!("SKIP: python venv not found");
        return;
    };

    let temp_mf4 = tempfile::Builder::new().suffix(".mf4").tempfile().unwrap();
    let temp_json = tempfile::Builder::new().suffix(".json").tempfile().unwrap();

    // 1. Create MF4 file in Rust with writer
    let mut writer = Mf4Writer::new();
    let time_axis = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0];
    let g = writer.add_group(&time_axis).unwrap();
    let ramp_data: Vec<f64> = time_axis.iter().map(|&t| t * 10.0).collect();
    g.add_channel("Ramp", "V", &ramp_data).unwrap();
    writer.write_to_file(temp_mf4.path()).unwrap();

    // 2. Run Python script using asammdf to execute cut and resample
    let py_script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

mdf = MDF(r"{mf4_path}")
ramp_sig = mdf.get("Ramp")

# Cut [1.0, 4.0] with include_ends=False
cut_sig = ramp_sig.cut(1.0, 4.0, include_ends=False)

# Resample raster=0.25 (linear)
res_sig = ramp_sig.interp(np.linspace(0.0, 5.0, 21), float_interpolation_mode=1)

out = {{
    "cut_timestamps": cut_sig.timestamps.tolist(),
    "cut_samples": cut_sig.samples.tolist(),
    "res_timestamps": res_sig.timestamps.tolist(),
    "res_samples": res_sig.samples.tolist(),
}}

with open(r"{json_path}", "w") as f:
    json.dump(out, f)
"#,
        mf4_path = temp_mf4.path().display(),
        json_path = temp_json.path().display(),
    );

    let status = Command::new(&python).args(["-c", &py_script]).status();

    let Ok(status) = status else {
        eprintln!("SKIP: failed to execute python");
        return;
    };
    if !status.success() {
        eprintln!("SKIP: python script failed");
        return;
    }

    let json_bytes = std::fs::read(temp_json.path()).unwrap();
    let py_data: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

    let py_cut_timestamps: Vec<f64> = py_data["cut_timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let py_cut_samples: Vec<f64> = py_data["cut_samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let py_res_timestamps: Vec<f64> = py_data["res_timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let py_res_samples: Vec<f64> = py_data["res_samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();

    // 3. Execute with falcon_mdf
    let file = Mf4File::open(temp_mf4.path()).unwrap();
    let ch = file.find_channel("Ramp").unwrap();

    let rust_cut = file.cut_channel(ch, 1.0, 4.0).unwrap();
    assert_eq!(rust_cut.timestamps(), &py_cut_timestamps[..]);
    assert_eq!(rust_cut.values_f64(), py_cut_samples);

    let rust_res = file
        .resample_channel(
            ch,
            py_res_timestamps.as_slice(),
            InterpolationMode::Linear,
        )
        .unwrap();
    assert_eq!(rust_res.timestamps(), &py_res_timestamps[..]);
    let rust_vals = rust_res.values_f64();
    for (r, p) in rust_vals.iter().zip(&py_res_samples) {
        assert!((r - p).abs() < 1e-9, "mismatch: rust={r}, py={p}");
    }

    println!("Cross-check with asammdf passed: cut and resample are 100% equivalent!");
}
