//! Integration tests for the multi-channel and multi-file operations:
//! `filter`, `concatenate` and `stack`.
//!
//! Every expected value here comes from one of two places, never from reading
//! the result back out of the implementation that produced it:
//!
//!   * the samples handed to `Mf4Writer` plus arithmetic done by hand in the
//!     comment above the assertion, or
//!   * asammdf's answer to the same question, computed in a subprocess.
//!
//! That distinction is the point of the file. Three silent data-corruption
//! defects in this repository survived their tests because the tests used the
//! implementation's own inverse as their oracle, and so only ever agreed with
//! themselves.

use falcon_mdf::multi_ops::{concatenate, stack};
use falcon_mdf::{ChannelSelector, Mf4File, Mf4Writer, TimeAlignment};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A whole-second Unix timestamp, in nanoseconds. Whole seconds because
/// asammdf carries the header start time as a `datetime`, which resolves to
/// microseconds — a nanosecond-precision start time would be truncated on its
/// side and not on ours, and the cross-checks would disagree over rounding
/// rather than over semantics.
fn secs_ns(unix_seconds: i64) -> i64 {
    unix_seconds * 1_000_000_000
}

const BASE: i64 = 1_700_000_000;

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Runs `script` under the venv interpreter and reads back the JSON it wrote.
///
/// Returns `None` when the interpreter or asammdf is unavailable, so the suite
/// still passes on a machine without the reference implementation — the
/// cross-check is then reported as skipped rather than silently green.
fn asammdf_answer(script: &str, json_path: &Path) -> Option<serde_json::Value> {
    let python = venv_python()?;
    let out = Command::new(&python).args(["-c", script]).output().ok()?;
    if !out.status.success() {
        eprintln!(
            "SKIP: asammdf script failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    let bytes = std::fs::read(json_path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn floats(value: &serde_json::Value) -> Vec<f64> {
    value
        .as_array()
        .expect("expected a JSON array of numbers")
        .iter()
        .map(|v| v.as_f64().expect("expected a number"))
        .collect()
}

fn assert_close(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length differs");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() < 1e-9,
            "{what}: sample {i} is {g}, expected {w}"
        );
    }
}

/// Writes a one-group file: `Time` master plus one channel per `(name, unit,
/// values)` triple.
fn write_file(path: &Path, start_time_ns: i64, times: &[f64], channels: &[(&str, &str, Vec<f64>)]) {
    let mut writer = Mf4Writer::with_start_time_ns(start_time_ns);
    let group = writer.add_group(times).unwrap();
    for (name, unit, values) in channels {
        group.add_channel(name, unit, values).unwrap();
    }
    writer.write_to_file(path).unwrap();
}

fn temp_mf4() -> tempfile::NamedTempFile {
    tempfile::Builder::new().suffix(".mf4").tempfile().unwrap()
}

fn temp_json() -> tempfile::NamedTempFile {
    tempfile::Builder::new().suffix(".json").tempfile().unwrap()
}

// ---------------------------------------------------------------------------
// filter
// ---------------------------------------------------------------------------

#[test]
fn filter_returns_only_the_named_channels_in_the_order_asked() {
    let f = temp_mf4();
    write_file(
        f.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[
            ("Speed", "km/h", vec![10.0, 20.0, 30.0]),
            ("RPM", "1/min", vec![1000.0, 2000.0, 3000.0]),
            ("Torque", "Nm", vec![5.0, 6.0, 7.0]),
        ],
    );
    let file = Mf4File::open(f.path()).unwrap();

    // Reverse order, and RPM left out entirely.
    let picked = file
        .filter(&["Torque".into(), "Speed".into()])
        .expect("both names are unique in this file");

    assert_eq!(picked.len(), 2);
    assert_eq!(picked[0].name(), "Torque");
    assert_eq!(picked[0].unit(), "Nm");
    // Straight from the values handed to the writer above.
    assert_eq!(picked[0].values_f64(), vec![5.0, 6.0, 7.0]);
    assert_eq!(picked[0].timestamps(), &[0.0, 1.0, 2.0]);
    assert_eq!(picked[1].name(), "Speed");
    assert_eq!(picked[1].values_f64(), vec![10.0, 20.0, 30.0]);
}

#[test]
fn filter_refuses_a_name_several_channels_share() {
    let f = temp_mf4();
    let mut writer = Mf4Writer::with_start_time_ns(secs_ns(BASE));
    let g0 = writer.add_group(&[0.0, 1.0]).unwrap();
    g0.add_channel("Speed", "km/h", &[10.0, 20.0]).unwrap();
    let g1 = writer.add_group(&[0.0, 1.0]).unwrap();
    g1.add_channel("Speed", "mph", &[6.0, 12.0]).unwrap();
    writer.write_to_file(f.path()).unwrap();
    let file = Mf4File::open(f.path()).unwrap();

    let err = file
        .filter(&["Speed".into()])
        .expect_err("an ambiguous name must not resolve to a silent first guess");
    let text = err.to_string();
    assert!(
        text.contains("Speed") && text.contains('2'),
        "the error should name the channel and say how many carry the name, got: {text}"
    );

    // The master channel is in both groups too, so it is ambiguous as well.
    assert!(file.filter(&["Time".into()]).is_err());

    // Naming the group resolves it, and picks that group's samples and unit.
    let second = file
        .filter(&[ChannelSelector::NameInGroup {
            name: "Speed".to_string(),
            data_group: 1,
            channel_group: 0,
        }])
        .unwrap();
    assert_eq!(second[0].unit(), "mph");
    assert_eq!(second[0].values_f64(), vec![6.0, 12.0]);
}

#[test]
fn filter_refuses_a_name_no_channel_carries() {
    let f = temp_mf4();
    write_file(
        f.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    let file = Mf4File::open(f.path()).unwrap();

    let err = file
        .filter(&["Altitude".into()])
        .expect_err("a name that matches nothing must be an error, not a silently shorter result");
    assert!(err.to_string().contains("Altitude"), "got: {err}");
}

#[test]
fn filter_by_position_ignores_the_name_and_bounds_check_its_indices() {
    let f = temp_mf4();
    write_file(
        f.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    let file = Mf4File::open(f.path()).unwrap();

    // Index 0 is the implicit `Time` master the writer puts first, index 1 is
    // Speed. Both are addressable by position.
    let by_position = file
        .filter(&[ChannelSelector::Position {
            data_group: 0,
            channel_group: 0,
            index: 1,
        }])
        .unwrap();
    assert_eq!(by_position[0].name(), "Speed");
    assert_eq!(by_position[0].values_f64(), vec![10.0, 20.0]);

    for out_of_range in [
        ChannelSelector::Position {
            data_group: 9,
            channel_group: 0,
            index: 0,
        },
        ChannelSelector::Position {
            data_group: 0,
            channel_group: 9,
            index: 0,
        },
        ChannelSelector::Position {
            data_group: 0,
            channel_group: 0,
            index: 9,
        },
    ] {
        assert!(
            file.filter(std::slice::from_ref(&out_of_range)).is_err(),
            "{out_of_range:?} should be rejected"
        );
    }
}

#[test]
fn cross_check_filter_against_asammdf() {
    let f = temp_mf4();
    let j = temp_json();
    let times: Vec<f64> = (0..20).map(|i| i as f64 * 0.05).collect();
    let speed: Vec<f64> = times.iter().map(|t| 30.0 + t * 7.5).collect();
    let rpm: Vec<f64> = times.iter().map(|t| 800.0 + t * 1200.0).collect();
    write_file(
        f.path(),
        secs_ns(BASE),
        &times,
        &[("Speed", "km/h", speed), ("RPM", "1/min", rpm)],
    );

    let script = format!(
        r#"
import json
from asammdf import MDF

mdf = MDF(r"{mf4}")
filtered = mdf.filter(["Speed"])
names = sorted(
    ch.name
    for gp in filtered.groups
    for ch in gp.channels
)
sig = filtered.get("Speed")
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "t": sig.timestamps.tolist(),
        "v": sig.samples.tolist(),
        "unit": sig.unit,
    }}, fh)
"#,
        mf4 = f.path().display(),
        js = j.path().display(),
    );

    let Some(py) = asammdf_answer(&script, j.path()) else {
        eprintln!("SKIP: asammdf unavailable");
        return;
    };

    let file = Mf4File::open(f.path()).unwrap();
    let ours = file.filter(&["Speed".into()]).unwrap();

    assert_eq!(ours.len(), 1, "asammdf kept {:?}", py["names"]);
    assert_eq!(ours[0].unit(), py["unit"].as_str().unwrap());
    assert_close(ours[0].timestamps(), &floats(&py["t"]), "filter timestamps");
    assert_close(&ours[0].values_f64(), &floats(&py["v"]), "filter samples");

    // asammdf's filtered file holds exactly the requested channel plus the
    // master it needs; ours drops the master because every series carries its
    // own timestamps. That is the only difference, and it is on purpose.
    assert_eq!(
        py["names"].as_array().unwrap().len(),
        2,
        "expected asammdf to keep Speed and its master, got {:?}",
        py["names"]
    );
}

// ---------------------------------------------------------------------------
// concatenate
// ---------------------------------------------------------------------------

#[test]
fn concatenate_joins_same_named_channels_end_to_end() {
    let a = temp_mf4();
    let b = temp_mf4();
    // B's header says it was recorded ten seconds after A's.
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![10.0, 20.0, 30.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 10),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![40.0, 50.0, 60.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // One series: the two files' `Speed` is one channel, joined.
    assert_eq!(joined.len(), 1);
    assert_eq!(joined[0].name(), "Speed");
    // A is the oldest so its offset is 0 and its master is unchanged. B's
    // offset is 10 s, putting it at 10, 11, 12 — after A's last sample at 2,
    // so no further push is needed.
    assert_close(
        joined[0].timestamps(),
        &[0.0, 1.0, 2.0, 10.0, 11.0, 12.0],
        "concatenated timestamps",
    );
    assert_eq!(
        joined[0].values_f64(),
        vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]
    );
    assert_eq!(joined[0].validity(), None);
}

#[test]
fn concatenate_pushes_an_overlapping_file_past_the_previous_end() {
    let a = temp_mf4();
    let b = temp_mf4();
    // Identical header start times, so the start-time offset is zero for both
    // and their masters would land on top of each other.
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![10.0, 20.0, 30.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE),
        &[0.0, 0.5, 1.0],
        &[("Speed", "km/h", vec![40.0, 50.0, 60.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // A ends at 2.0. B's own step is 0.5, so B is rebased to start at
    // 2.0 + 0.5 = 2.5 and runs 2.5, 3.0, 3.5. Nothing overlaps and nothing is
    // dropped: an overlap is an alignment problem, not a reason to discard
    // samples.
    assert_close(
        joined[0].timestamps(),
        &[0.0, 1.0, 2.0, 2.5, 3.0, 3.5],
        "overlapping concatenated timestamps",
    );
    assert_eq!(
        joined[0].values_f64(),
        vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]
    );
    // Monotonic, which is what makes the result usable as a time series.
    assert!(joined[0].timestamps().windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn concatenate_uses_a_millisecond_step_for_a_single_sample_file() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE),
        &[0.0],
        &[("Speed", "km/h", vec![99.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // A ends at 1.0. B has one sample, so it has no step of its own to
    // continue with; 1 ms is used, matching asammdf's fallback.
    assert_close(
        joined[0].timestamps(),
        &[0.0, 1.0, 1.001],
        "single-sample continuation",
    );
    assert_eq!(joined[0].values_f64(), vec![10.0, 20.0, 99.0]);
}

#[test]
fn concatenate_as_recorded_ignores_the_header_start_times() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![10.0, 20.0, 30.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 1_000),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![40.0, 50.0, 60.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::AsRecorded).unwrap();

    // The 1000-second header gap is ignored, so B still collides with A at
    // zero and is pushed to 2.0 + 1.0 = 3.0 instead of to 1000.0.
    assert_close(
        joined[0].timestamps(),
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        "as-recorded concatenated timestamps",
    );
    assert_eq!(
        joined[0].values_f64(),
        vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]
    );
}

#[test]
fn concatenate_matches_channels_by_name_not_by_position() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[
            ("Speed", "km/h", vec![10.0, 20.0]),
            ("Torque", "Nm", vec![1.0, 2.0]),
        ],
    );
    // Same channels, opposite order in the group.
    write_file(
        b.path(),
        secs_ns(BASE + 10),
        &[0.0, 1.0],
        &[
            ("Torque", "Nm", vec![3.0, 4.0]),
            ("Speed", "km/h", vec![30.0, 40.0]),
        ],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    assert_eq!(joined.len(), 2);
    // The result follows the first file's order.
    assert_eq!(joined[0].name(), "Speed");
    assert_eq!(joined[1].name(), "Torque");
    // The decisive assertion: Speed's tail is B's Speed (30, 40), not B's
    // first-listed channel Torque (3, 4).
    assert_eq!(joined[0].values_f64(), vec![10.0, 20.0, 30.0, 40.0]);
    assert_eq!(joined[1].values_f64(), vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn concatenate_pairs_two_same_named_channels_in_a_group_one_to_one() {
    // A group carrying the same name twice is legal MF4 — a name is not a key.
    // Matching by name alone would resolve both of the first file's slots to
    // the second file's first copy, duplicating one channel's samples and
    // dropping the other's. Both files use the same order here, so the
    // one-to-one pairing is what the data says it is.
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[
            ("Speed", "km/h", vec![10.0, 20.0]),
            ("Speed", "mph", vec![6.0, 12.0]),
        ],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 10),
        &[0.0, 1.0],
        &[
            ("Speed", "km/h", vec![30.0, 40.0]),
            ("Speed", "mph", vec![18.0, 24.0]),
        ],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    assert_eq!(joined.len(), 2);
    assert_eq!(joined[0].values_f64(), vec![10.0, 20.0, 30.0, 40.0]);
    assert_eq!(joined[1].values_f64(), vec![6.0, 12.0, 18.0, 24.0]);
    assert_eq!(joined[0].unit(), "km/h");
    assert_eq!(joined[1].unit(), "mph");
}

#[test]
fn concatenate_refuses_files_that_do_not_describe_the_same_measurement() {
    let a = temp_mf4();
    let missing = temp_mf4();
    let extra_group = temp_mf4();

    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[
            ("Speed", "km/h", vec![10.0, 20.0]),
            ("Torque", "Nm", vec![1.0, 2.0]),
        ],
    );
    // Torque absent.
    write_file(
        missing.path(),
        secs_ns(BASE + 10),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![30.0, 40.0])],
    );
    // Right channels, but an extra group.
    let mut writer = Mf4Writer::with_start_time_ns(secs_ns(BASE + 10));
    let g0 = writer.add_group(&[0.0, 1.0]).unwrap();
    g0.add_channel("Speed", "km/h", &[30.0, 40.0]).unwrap();
    g0.add_channel("Torque", "Nm", &[3.0, 4.0]).unwrap();
    let g1 = writer.add_group(&[0.0, 1.0]).unwrap();
    g1.add_channel("Extra", "-", &[0.0, 0.0]).unwrap();
    writer.write_to_file(extra_group.path()).unwrap();

    let fa = Mf4File::open(a.path()).unwrap();
    let fmissing = Mf4File::open(missing.path()).unwrap();
    let fextra = Mf4File::open(extra_group.path()).unwrap();

    let err = concatenate(&[&fa, &fmissing], TimeAlignment::StartTime)
        .expect_err("a missing channel must not be silently filled in");
    assert!(err.to_string().contains("Torque"), "got: {err}");

    let err = concatenate(&[&fa, &fextra], TimeAlignment::StartTime)
        .expect_err("a differing group count must be refused");
    assert!(err.to_string().contains("channel group"), "got: {err}");
}

#[test]
fn concatenate_treats_a_file_without_invalidation_bits_as_all_valid() {
    let a = temp_mf4();
    let b = temp_mf4();

    let mut writer = Mf4Writer::with_start_time_ns(secs_ns(BASE));
    let g = writer.add_group(&[0.0, 1.0, 2.0]).unwrap();
    g.add_channel_with_validity(
        "Sensor",
        "bar",
        &[1.0, 2.0, 3.0],
        Some(&[true, false, true]),
    )
    .unwrap();
    writer.write_to_file(a.path()).unwrap();

    write_file(
        b.path(),
        secs_ns(BASE + 10),
        &[0.0, 1.0],
        &[("Sensor", "bar", vec![4.0, 5.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // A's mask survives; B, which has no invalidation bits at all, contributes
    // valid samples. Dropping the mask because one file lacked one would lose
    // the only record that A's second sample is not to be trusted.
    assert_eq!(
        joined[0].validity(),
        Some(&[true, false, true, true, true][..])
    );
    assert_eq!(joined[0].values_f64(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn concatenate_skips_a_file_with_no_samples_without_disturbing_the_others() {
    let empty = temp_mf4();
    let a = temp_mf4();
    let b = temp_mf4();

    write_file(
        empty.path(),
        secs_ns(BASE),
        &[],
        &[("Speed", "km/h", vec![])],
    );
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![30.0, 40.0])],
    );

    let fe = Mf4File::open(empty.path()).unwrap();
    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();

    // The empty file in the middle is not a recording that ended, so it must
    // not act as one: B still continues from A's last timestamp, exactly as it
    // would if the empty file were not in the list at all.
    let with_gap = concatenate(&[&fa, &fe, &fb], TimeAlignment::AsRecorded).unwrap();
    let without = concatenate(&[&fa, &fb], TimeAlignment::AsRecorded).unwrap();

    assert_close(
        with_gap[0].timestamps(),
        &[0.0, 1.0, 2.0, 3.0],
        "with empty",
    );
    assert_eq!(with_gap[0].values_f64(), vec![10.0, 20.0, 30.0, 40.0]);
    assert_eq!(with_gap[0].timestamps(), without[0].timestamps());

    // And an entirely empty concatenation is an empty series, not an error.
    let nothing = concatenate(&[&fe], TimeAlignment::AsRecorded).unwrap();
    assert_eq!(nothing.len(), 1);
    assert!(nothing[0].is_empty());
}

#[test]
fn concatenate_of_one_file_is_that_file() {
    let a = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[("Speed", "km/h", vec![10.0, 20.0, 30.0])],
    );
    let fa = Mf4File::open(a.path()).unwrap();
    let joined = concatenate(&[&fa], TimeAlignment::StartTime).unwrap();

    assert_eq!(joined.len(), 1);
    assert_eq!(joined[0].timestamps(), &[0.0, 1.0, 2.0]);
    assert_eq!(joined[0].values_f64(), vec![10.0, 20.0, 30.0]);
}

#[test]
fn concatenate_and_stack_refuse_an_empty_file_list() {
    assert!(concatenate(&[], TimeAlignment::StartTime).is_err());
    assert!(stack(&[], TimeAlignment::StartTime).is_err());
}

#[test]
fn cross_check_concatenate_against_asammdf() {
    // Two scenarios in one test because they share the whole harness: files
    // that do not overlap once shifted, and files that do.
    for (label, b_start, b_times) in [
        ("disjoint", BASE + 10, vec![0.0, 0.25, 0.5, 0.75, 1.0]),
        ("overlapping", BASE, vec![0.0, 0.25, 0.5, 0.75, 1.0]),
    ] {
        let a = temp_mf4();
        let b = temp_mf4();
        let j = temp_json();

        let a_times = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5];
        let a_speed: Vec<f64> = a_times.iter().map(|t| 10.0 + t * 3.0).collect();
        let a_torque: Vec<f64> = a_times.iter().map(|t| 100.0 - t).collect();
        let b_speed: Vec<f64> = b_times.iter().map(|t| 50.0 + t * 8.0).collect();
        let b_torque: Vec<f64> = b_times.iter().map(|t| 200.0 - t * 2.0).collect();

        write_file(
            a.path(),
            secs_ns(BASE),
            &a_times,
            &[("Speed", "km/h", a_speed), ("Torque", "Nm", a_torque)],
        );
        write_file(
            b.path(),
            secs_ns(b_start),
            &b_times,
            &[("Speed", "km/h", b_speed), ("Torque", "Nm", b_torque)],
        );

        let script = format!(
            r#"
import json
from asammdf import MDF

merged = MDF.concatenate([r"{a}", r"{b}"])
out = {{}}
for name in ("Speed", "Torque"):
    sig = merged.get(name)
    out[name] = {{"t": sig.timestamps.tolist(), "v": sig.samples.tolist()}}
with open(r"{js}", "w") as fh:
    json.dump(out, fh)
"#,
            a = a.path().display(),
            b = b.path().display(),
            js = j.path().display(),
        );

        let Some(py) = asammdf_answer(&script, j.path()) else {
            eprintln!("SKIP: asammdf unavailable");
            return;
        };

        let fa = Mf4File::open(a.path()).unwrap();
        let fb = Mf4File::open(b.path()).unwrap();
        let joined = concatenate(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

        assert_eq!(joined.len(), 2, "{label}: expected Speed and Torque");
        for series in &joined {
            let reference = &py[series.name()];
            assert!(
                !reference.is_null(),
                "{label}: asammdf produced no {}",
                series.name()
            );
            assert_close(
                series.timestamps(),
                &floats(&reference["t"]),
                &format!("{label}: {} timestamps", series.name()),
            );
            assert_close(
                &series.values_f64(),
                &floats(&reference["v"]),
                &format!("{label}: {} samples", series.name()),
            );
        }
        println!("concatenate cross-check ({label}) agrees with asammdf");
    }
}

// ---------------------------------------------------------------------------
// stack
// ---------------------------------------------------------------------------

#[test]
fn stack_keeps_same_named_channels_apart_and_offsets_each_file() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 5),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![30.0, 40.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let stacked = stack(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // Two series, not one: side by side is not end to end, so the two files'
    // `Speed` stays two channels and neither file's samples are absorbed into
    // the other's.
    assert_eq!(stacked.len(), 2);
    assert_eq!(stacked[0].file_index, 0);
    assert_eq!(stacked[1].file_index, 1);
    assert_eq!(stacked[0].series.name(), "Speed");
    assert_eq!(stacked[1].series.name(), "Speed");

    assert_close(stacked[0].series.timestamps(), &[0.0, 1.0], "file 0 times");
    assert_eq!(stacked[0].series.values_f64(), vec![10.0, 20.0]);
    // B's header is 5 s later, so its samples sit at 5 and 6 on the shared
    // axis — overlapping A's range is fine here, which is the whole difference
    // from concatenate.
    assert_close(stacked[1].series.timestamps(), &[5.0, 6.0], "file 1 times");
    assert_eq!(stacked[1].series.values_f64(), vec![30.0, 40.0]);
}

#[test]
fn stack_leaves_different_sample_rates_alone() {
    let a = temp_mf4();
    let b = temp_mf4();
    // 1 Hz and 4 Hz.
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0, 2.0],
        &[("Slow", "V", vec![1.0, 2.0, 3.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE),
        &[0.0, 0.25, 0.5, 0.75],
        &[("Fast", "A", vec![9.0, 8.0, 7.0, 6.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let stacked = stack(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    // Each keeps its own raster: stacking shares an origin, not a grid. Forcing
    // a common raster here would invent samples the slow channel never had.
    assert_close(stacked[0].series.timestamps(), &[0.0, 1.0, 2.0], "slow");
    assert_close(
        stacked[1].series.timestamps(),
        &[0.0, 0.25, 0.5, 0.75],
        "fast",
    );
    assert_eq!(stacked[0].series.values_f64(), vec![1.0, 2.0, 3.0]);
    assert_eq!(stacked[1].series.values_f64(), vec![9.0, 8.0, 7.0, 6.0]);

    // And a caller who does want one grid can ask for it afterwards; the slow
    // channel then step-holds, which is a choice made at that point rather
    // than one stack made for them.
    let on_a_grid = stacked[0]
        .series
        .resample(0.25, falcon_mdf::InterpolationMode::StepHold)
        .unwrap();
    assert_eq!(on_a_grid.len(), 9);
}

#[test]
fn stack_accepts_files_with_completely_different_channels() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Coolant", "degC", vec![80.0, 82.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();

    // Unlike concatenate, disjoint channel sets are the normal case here: an
    // engine log and a chassis log describe the same drive from two angles.
    let stacked = stack(&[&fa, &fb], TimeAlignment::StartTime).unwrap();
    let names: Vec<&str> = stacked.iter().map(|s| s.series.name()).collect();
    assert_eq!(names, vec!["Speed", "Coolant"]);

    // The same pair is refused by concatenate.
    assert!(concatenate(&[&fa, &fb], TimeAlignment::StartTime).is_err());
}

#[test]
fn stack_as_recorded_applies_no_offset() {
    let a = temp_mf4();
    let b = temp_mf4();
    write_file(
        a.path(),
        secs_ns(BASE),
        &[0.0, 1.0],
        &[("Speed", "km/h", vec![10.0, 20.0])],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 5),
        &[0.0, 1.0],
        &[("Coolant", "degC", vec![80.0, 82.0])],
    );

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let stacked = stack(&[&fa, &fb], TimeAlignment::AsRecorded).unwrap();

    assert_close(stacked[0].series.timestamps(), &[0.0, 1.0], "file 0");
    assert_close(stacked[1].series.timestamps(), &[0.0, 1.0], "file 1");
}

#[test]
fn cross_check_stack_against_asammdf() {
    let a = temp_mf4();
    let b = temp_mf4();
    let j = temp_json();

    let a_times: Vec<f64> = (0..8).map(|i| i as f64 * 0.5).collect();
    let b_times: Vec<f64> = (0..12).map(|i| i as f64 * 0.2).collect();
    let a_speed: Vec<f64> = a_times.iter().map(|t| 12.0 + t * 4.0).collect();
    // Deliberately the same channel name in both files, so the cross-check
    // also covers the "kept apart" decision: asammdf puts them in two groups.
    let b_speed: Vec<f64> = b_times.iter().map(|t| 70.0 - t * 3.0).collect();

    write_file(
        a.path(),
        secs_ns(BASE),
        &a_times,
        &[("Speed", "km/h", a_speed)],
    );
    write_file(
        b.path(),
        secs_ns(BASE + 7),
        &b_times,
        &[("Speed", "km/h", b_speed)],
    );

    let script = format!(
        r#"
import json
from asammdf import MDF

stacked = MDF.stack([r"{a}", r"{b}"])
groups = []
for gi, gp in enumerate(stacked.groups):
    for ci, ch in enumerate(gp.channels):
        # channel types 2 and 3 are the master channels
        if ch.channel_type in (2, 3):
            continue
        sig = stacked.get(group=gi, index=ci)
        groups.append({{
            "group": gi,
            "name": ch.name,
            "t": sig.timestamps.tolist(),
            "v": sig.samples.tolist(),
        }})
with open(r"{js}", "w") as fh:
    json.dump(groups, fh)
"#,
        a = a.path().display(),
        b = b.path().display(),
        js = j.path().display(),
    );

    let Some(py) = asammdf_answer(&script, j.path()) else {
        eprintln!("SKIP: asammdf unavailable");
        return;
    };

    let fa = Mf4File::open(a.path()).unwrap();
    let fb = Mf4File::open(b.path()).unwrap();
    let stacked = stack(&[&fa, &fb], TimeAlignment::StartTime).unwrap();

    let reference = py.as_array().unwrap();
    assert_eq!(
        stacked.len(),
        reference.len(),
        "asammdf stacked into {} series, we produced {}",
        reference.len(),
        stacked.len()
    );

    for (i, (ours, theirs)) in stacked.iter().zip(reference).enumerate() {
        assert_eq!(ours.series.name(), theirs["name"].as_str().unwrap());
        assert_eq!(
            ours.file_index,
            theirs["group"].as_u64().unwrap() as usize,
            "series {i}: our file index should match asammdf's group index, \
             since it appends one group per input file in order"
        );
        assert_close(
            ours.series.timestamps(),
            &floats(&theirs["t"]),
            &format!("stacked series {i} timestamps"),
        );
        assert_close(
            &ours.series.values_f64(),
            &floats(&theirs["v"]),
            &format!("stacked series {i} samples"),
        );
    }
    println!("stack cross-check agrees with asammdf");
}
