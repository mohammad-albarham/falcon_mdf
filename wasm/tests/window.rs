//! Native tests for the viewer's Rust-side work: windowed decimation
//! (`decimate_window`), CSV export (`signal_csv`), and the one-call channel
//! metadata (`channels`). `signal_arrays`/`signal_window` build JS objects and
//! only exist on wasm32; here their logic is covered through the pure
//! `decimate_window` they delegate to, plus one test pinning the native
//! fallback.
//!
//! `decimate_window` must survive whatever a malformed or hostile file puts in
//! a master channel, so the non-monotonic tests below assert both survival of
//! the spike and termination — a hang is a failure of the same guarantee.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf::write::Mf4Writer;
use falcon_mdf_wasm::{decimate_window, series_csv, WasmMf4File};

/// A writer file with `n` samples of ` ramp ` on one channel — the fixture for
/// the API-level CSV tests.
fn written_file(name: &str, unit: &str, timestamps: &[f64], values: &[f64]) -> WasmMf4File {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(timestamps).expect("add group");
    group.add_channel(name, unit, values).expect("add channel");
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write MF4");
    WasmMf4File::new(bytes).expect("open")
}

// ---------------------------------------------------------------- decimation

#[test]
fn a_column_keeps_first_min_max_last_in_source_order() {
    // Five samples squeezed into one column (max_points 4 → n_columns 1):
    // the column must keep its first sample, its min, its max and its last
    // sample, emitted in source (time) order.
    let times = [0.0, 0.1, 0.2, 0.3, 0.4];
    let values = [5.0, -3.0, 8.0, 1.0, 7.0];
    let (ts, vs) = decimate_window(&times, &values, 0.0, 0.5, 4);
    assert_eq!(ts, vec![0.0, 0.1, 0.2, 0.4]);
    assert_eq!(vs, vec![5.0, -3.0, 8.0, 7.0]);

    // Min after max in the source: the pair comes back in the order they were
    // sampled, not min-then-max unconditionally.
    let values = [5.0, 8.0, -3.0, 1.0, 7.0];
    let (_, vs) = decimate_window(&times, &values, 0.0, 0.5, 4);
    assert_eq!(vs, vec![5.0, 8.0, -3.0, 7.0]);

    // A flat column collapses to a single point rather than four duplicates.
    let values = [2.0; 5];
    let (ts, vs) = decimate_window(&times, &values, 0.0, 0.5, 4);
    assert_eq!(ts, vec![0.0, 0.4]);
    assert_eq!(vs, vec![2.0, 2.0]);
}

#[test]
fn a_spike_survives_a_non_monotonic_axis() {
    // 600 samples whose master is not monotonic: a stretch of duplicated
    // timestamps, plus one timestamp that steps backwards. The walk is over
    // file order, so it must still terminate and still keep the spike — it is
    // an extreme of whatever column it lands in, wherever that column sits.
    let mut times: Vec<f64> = (0..600).map(|i| i as f64 * 0.5).collect();
    for t in times.iter_mut().take(340).skip(300) {
        *t = 299.5 * 0.5 + 75.0; // 40 samples sharing one timestamp
    }
    times[500] = times[502] - 0.25; // a step backwards
    let mut values = vec![1.0; 600];
    values[310] = 1000.0; // the spike, inside the duplicated stretch

    let (ts, vs) = decimate_window(&times, &values, f64::NEG_INFINITY, f64::INFINITY, 100);
    assert!(
        vs.iter().any(|&v| v == 1000.0),
        "the spike must survive decimation: {vs:?}"
    );
    assert!(!ts.is_empty(), "points must still come out");
    // Budget: 100 points + the 1.5× hard cap for degenerate columns.
    let ts_len = ts.len();
    assert!(
        ts.len() <= 150,
        "a 100-point budget may not return {ts_len} points"
    );
    assert_eq!(ts.len(), vs.len());
}

#[test]
fn identical_timestamps_far_beyond_a_column_width_do_not_explode() {
    // The whole signal shares one timestamp: columns degenerate (their width
    // is 0 against this span), so the output is the run's first/min/max/last
    // — bounded, spike-safe, terminating.
    let times = vec![1.7e9; 1000];
    let values: Vec<f64> = (0..1000).map(|i| i as f64).collect();
    let (ts, vs) = decimate_window(&times, &values, 1.7e9, 1.7e9 + 1e-6, 200);
    assert!(
        vs.len() <= 4,
        "one degenerate column, at most four points: {vs:?}"
    );
    assert!(vs.contains(&0.0), "the first sample is kept: {vs:?}");
    assert!(vs.contains(&999.0), "the max is kept: {vs:?}");
}

#[test]
fn a_sample_exactly_on_a_bucket_edge_lands_in_the_later_bucket() {
    // Window [0, 8], max_points 8 → n_columns 2 → column width 4.0. The
    // sample at t = 4.0 computes col_index 4.0 / 4.0 = 1, so it opens column
    // 1; the sample at t = 8.0 lands one past the nominal columns (col_end
    // of column 1 is 8.0, exclusive), exactly like the GUI decimator.
    let times: Vec<f64> = (0..=8).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..=8).map(|i| i as f64).collect();
    let (ts, vs) = decimate_window(&times, &values, 0.0, 8.0, 8);
    // Column 0 [0, 4): first=min=0 at t=0, last=max=3 at t=3.
    // Column 1 [4, 8): first=min=4 at t=4, last=max=7 at t=7.
    // Tail column [8, 12): the sample at t=8 alone.
    assert_eq!(ts, vec![0.0, 3.0, 4.0, 7.0, 8.0]);
    assert_eq!(vs, vec![0.0, 3.0, 4.0, 7.0, 8.0]);
}

#[test]
fn an_empty_window_yields_no_points() {
    let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let values = times.clone();

    let (ts, vs) = decimate_window(&times, &values, 10.0, 20.0, 100);
    assert!(ts.is_empty() && vs.is_empty(), "window past the data");

    let (ts, vs) = decimate_window(&times, &values, 7.0, 3.0, 100);
    assert!(ts.is_empty() && vs.is_empty(), "reversed window");

    let (ts, vs) = decimate_window(&times, &values, 0.0, 5.0, 0);
    assert!(
        ts.is_empty() && vs.is_empty(),
        "a zero-point budget draws nothing"
    );

    let (ts, vs) = decimate_window(&[], &[], f64::NEG_INFINITY, f64::INFINITY, 100);
    assert!(ts.is_empty() && vs.is_empty(), "empty series");

    let (ts, vs) = decimate_window(&times, &values, f64::NAN, 5.0, 100);
    assert!(
        ts.is_empty() && vs.is_empty(),
        "a NaN bound is not a window"
    );
}

#[test]
fn a_window_within_the_budget_returns_the_samples_untouched() {
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let values = vec![10.0, 20.0, 5.0, 15.0];
    let (ts, vs) = decimate_window(&times, &values, 0.0, 3.0, 100);
    assert_eq!(ts, times);
    assert_eq!(vs, values);
}

#[test]
fn non_finite_runs_collapse_to_one_nan_point_and_never_become_extremes() {
    // 400 valid samples, 100 NaN, 500 valid samples with a spike, budget 200.
    // The gap must come out as exactly one NaN point (a gap the line breaks
    // on), the garbage must not stretch anything, and the spike must survive.
    let n = 1000;
    let times: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let mut values = vec![1.0; n];
    for v in values.iter_mut().take(500).skip(400) {
        *v = f64::NAN;
    }
    values[700] = 999.0;

    let (ts, vs) = decimate_window(&times, &values, 0.0, 999.0, 200);
    let nans: Vec<f64> = vs.iter().copied().filter(|v| v.is_nan()).collect();
    assert_eq!(nans.len(), 1, "one gap run, one NaN point: {vs:?}");
    assert!(
        ts.len() < n && vs.len() < n,
        "the window must actually be decimated ({} points for {n} samples)",
        ts.len()
    );
    assert!(ts.len() <= 300, "hard cap is 1.5× the 200-point budget");
    assert!(vs.contains(&999.0), "the spike survives");
    let gap_t = ts[vs.iter().position(|v| v.is_nan()).expect("a NaN above")];
    assert!(
        (400.0..500.0).contains(&gap_t),
        "the NaN point carries a timestamp from the gap run: {gap_t}"
    );
    assert!(
        vs.iter().all(|v| v.is_nan() || *v == 1.0 || *v == 999.0),
        "no other values exist in the fixture: {vs:?}"
    );
}

#[test]
fn an_all_invalid_window_reports_the_gap() {
    let times: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let values = vec![f64::NAN; 100];
    let (ts, vs) = decimate_window(&times, &values, 0.0, 99.0, 10);
    assert_eq!(ts.len(), 1, "the whole gap run is one point: {ts:?}");
    assert!(vs[0].is_nan());
}

#[test]
fn the_budget_holds_on_a_large_signal_and_both_extremes_survive() {
    let n = 100_000;
    let times: Vec<f64> = (0..n).map(|i| i as f64 * 0.01).collect();
    let values: Vec<f64> = (0..n)
        .map(|i| (((i as f64) * 7.0).sin() * 50.0) + ((i % 997) as f64))
        .collect();
    let (min, max) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        });

    let (ts, vs) = decimate_window(&times, &values, 0.0, 999.99, 1000);
    assert!(
        ts.len() <= 1500,
        "hard cap: {} points for a 1000 budget",
        ts.len()
    );
    assert_eq!(ts.len(), vs.len());
    assert!(
        vs.iter().cloned().fold(f64::INFINITY, f64::min) <= min + 1e-9,
        "the global min must survive decimation"
    );
    assert!(
        vs.iter().cloned().fold(f64::NEG_INFINITY, f64::max) >= max - 1e-9,
        "the global max must survive decimation"
    );
    assert!(
        ts.windows(2).all(|w| w[0] <= w[1]),
        "output must stay time-ordered within each drawn run"
    );
}

#[test]
fn infinite_bounds_clamp_to_the_series_extent() {
    let times: Vec<f64> = (0..50).map(|i| 10.0 + i as f64).collect();
    let values: Vec<f64> = (0..50).map(|i| i as f64).collect();
    let (inf_ts, inf_vs) = decimate_window(&times, &values, f64::NEG_INFINITY, f64::INFINITY, 20);
    let (full_ts, full_vs) = decimate_window(&times, &values, 10.0, 59.0, 20);
    assert_eq!(inf_ts, full_ts);
    assert_eq!(inf_vs, full_vs);
}

#[test]
fn a_partial_window_decimates_only_what_it_shows() {
    // Zoomed to [100, 200) of a 0..1000 signal: no point outside the window,
    // and the window's own extremes survive.
    let times: Vec<f64> = (0..1000).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..1000).map(|i| (i % 23) as f64).collect();
    let (ts, vs) = decimate_window(&times, &values, 100.0, 200.0, 40);
    assert!(
        ts.first() >= Some(&100.0) && ts.last() <= Some(&200.0),
        "in window: {ts:?}"
    );
    let min = vs.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = vs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(min, 0.0, "the window's min (i % 23) survives");
    assert_eq!(max, 22.0, "the window's max survives");
}

// ---------------------------------------------------------------------- CSV

#[test]
fn csv_rows_match_the_window_exactly() {
    let timestamps = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5];
    let values = [1.0, 2.5, 3.0, 4.0, 5.0, 6.0];
    let file = written_file("Speed", "m/s", &timestamps, &values);

    let csv = file.signal_csv("Speed", 0.1, 0.35).expect("csv");
    let lines: Vec<&str> = csv.trim_end_matches('\n').split('\n').collect();
    assert_eq!(
        lines[0], "timestamp,Speed",
        "header names the channel, not a generic column"
    );
    // Samples at t = 0.1, 0.2, 0.3 fall in [0.1, 0.35]; header + 3 rows.
    assert_eq!(
        lines.len(),
        4,
        "row count must match the visible window: {csv}"
    );
    assert_eq!(lines[1], "0.1,2.5");
    assert_eq!(lines[2], "0.2,3");
    assert_eq!(lines[3], "0.3,4");

    // The full window (infinite bounds) covers every sample.
    let full = file
        .signal_csv("Speed", f64::NEG_INFINITY, f64::INFINITY)
        .expect("csv");
    assert_eq!(full.trim_end_matches('\n').split('\n').count(), 7);
}

#[test]
fn csv_writes_non_finite_values_as_empty_fields() {
    let timestamps = [0.0, 1.0, 2.0];
    let values = [1.25, f64::NAN, f64::INFINITY];
    let file = written_file("S", "", &timestamps, &values);
    let csv = file.signal_csv("S", 0.0, 2.0).expect("csv");
    let lines: Vec<&str> = csv.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines[1], "0,1.25");
    assert_eq!(lines[2], "1,", "NaN is an empty field, never the text NaN");
    assert_eq!(lines[3], "2,", "infinity is an empty field too");
}

#[test]
fn csv_quotes_a_channel_name_that_looks_like_two_fields() {
    let out = series_csv(&[0.0], &[1.0], "Engine, left");
    assert_eq!(out, "timestamp,\"Engine, left\"\n0,1\n");
    let plain = series_csv(&[0.0], &[1.0], "EngineSpeed");
    assert_eq!(plain, "timestamp,EngineSpeed\n0,1\n");
}

#[test]
fn csv_for_a_missing_channel_is_an_error_and_an_empty_window_is_a_header() {
    let file = written_file("Speed", "m/s", &[0.0, 1.0], &[1.0, 2.0]);
    assert!(
        file.signal_csv("Nope", 0.0, 1.0).is_err(),
        "a missing channel must be a thrown error, not an empty file"
    );
    let csv = file.signal_csv("Speed", 10.0, 20.0).expect("csv");
    assert_eq!(
        csv, "timestamp,Speed\n",
        "a window with no samples is just the header"
    );
}

#[test]
fn csv_over_the_canedge_reference_file_matches_its_json_signal() {
    let path = std::path::Path::new("test_data/mf4-sample-data-v2.1")
        .join("OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4");
    let Ok(bytes) = std::fs::read(&path) else {
        return; // corpus not fetched
    };
    let file = WasmMf4File::new(bytes).expect("open CANedge log");
    let names_json = file.channel_names().expect("names");
    let JsonVal::Array(names) = parse_json(&names_json).expect("valid names json") else {
        panic!("names is an array")
    };
    let names: Vec<String> = names
        .into_iter()
        .map(|v| match v {
            JsonVal::Str(s) => s,
            other => panic!("channel names are strings: {other:?}"),
        })
        .collect();
    let data_name = names
        .iter()
        .find(|n| !n.eq_ignore_ascii_case("t"))
        .expect("a data channel beside the master");

    // Cross-check against the JSON signal path: same samples, same order.
    let json = file.signal(data_name).expect("signal json");
    let parsed = parse_json(&json).expect("valid signal json");
    let JsonVal::Obj(fields) = parsed else {
        panic!("signal object")
    };
    let n_json = fields
        .iter()
        .find(|(k, _)| k == "timestamps")
        .map(|(_, v)| match v {
            JsonVal::Array(a) => a.len(),
            _ => 0,
        })
        .expect("timestamps array");

    let csv = file
        .signal_csv(data_name, f64::NEG_INFINITY, f64::INFINITY)
        .expect("csv");
    let rows = csv.trim_end_matches('\n').split('\n').count() - 1; // minus header
    assert_eq!(rows, n_json, "CSV rows must equal the JSON sample count");
}

// ----------------------------------------------------------------- channels

#[test]
fn channels_lists_metadata_for_every_channel_name() {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0]).expect("add group");
    group
        .add_channel("RPM", "1/min", &[1000.0, 2000.0])
        .expect("add channel");
    group
        .add_channel("Temp", "°C", &[20.0, 21.0])
        .expect("add channel");
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write");
    let file = WasmMf4File::new(bytes).expect("open");

    let parsed = parse_json(&file.channels().expect("channels json")).expect("valid json");
    let JsonVal::Array(entries) = parsed else {
        panic!("channels is an array")
    };

    let names_json = file.channel_names().expect("names");
    let JsonVal::Array(names) = parse_json(&names_json).expect("valid names json") else {
        panic!("names is an array")
    };
    assert_eq!(
        entries.len(),
        names.len(),
        "one channels() entry per channel_names() entry (masters included)"
    );
    let names: Vec<String> = names
        .into_iter()
        .map(|v| match v {
            JsonVal::Str(s) => s,
            other => panic!("channel names are strings: {other:?}"),
        })
        .collect();

    for entry in &entries {
        let JsonVal::Obj(fields) = entry else {
            panic!("entry object")
        };
        for key in ["name", "unit", "group", "description"] {
            assert!(
                fields.iter().any(|(k, _)| k == key),
                "every entry carries {key}: {fields:?}"
            );
        }
    }
    let rpm = entries.iter().find_map(|e| match e {
        JsonVal::Obj(fields)
            if fields
                .iter()
                .any(|(k, v)| k == "name" && *v == JsonVal::Str("RPM".into())) =>
        {
            Some(fields)
        }
        _ => None,
    });
    let Some(rpm) = rpm else {
        panic!("RPM entry present: {entries:?}")
    };
    assert!(rpm
        .iter()
        .any(|(k, v)| k == "unit" && *v == JsonVal::Str("1/min".into())));
}

#[test]
fn typed_array_endpoints_report_their_native_limit_as_errors() {
    // signal_arrays/signal_window build JS objects; on a non-wasm build there
    // is no JS runtime, so they must return Err instead of panicking — a wasm
    // panic would kill the whole module for every caller.
    let mut file = written_file("Speed", "m/s", &[0.0, 1.0], &[1.0, 2.0]);
    assert!(file.signal_arrays("Speed").is_err());
    assert!(file.signal_window("Speed", 0.0, 1.0, 10).is_err());
}
