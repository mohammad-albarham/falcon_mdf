//! Native tests for the text-channel path (plan 2.1): run-collapsed label
//! windows (`decimate_label_runs`), label distribution statistics
//! (`window_label_stats_json`), label CSV (`labels_csv`), and the kind
//! metadata that lets a viewer mark channels without decoding them.
//!
//! The scalar endpoints keep their `to_f64()` view of a text channel (all
//! NaN), so these tests also pin the honest replacement: `signal_text` and
//! `signal_stats` return the labels, and everything refuses what it cannot
//! say. Corpus fixtures are the Vector text-conversion files; their content
//! was pinned against asammdf (10 samples at 1 s, labels verified by hand).

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf_wasm::{decimate_label_runs, labels_csv, window_label_stats_json, WasmMf4File};

/// The corpus file's bytes, or `None` when the vendor corpus isn't fetched —
/// the same skip-silently stance as the other corpus tests in this crate.
/// Test binaries run with the package dir (`wasm/`) as their CWD, so the
/// workspace-root corpus is one level up; both spellings are tried so the
/// test stays honest about "missing" versus "misplaced".
fn corpus(name: &str) -> Option<WasmMf4File> {
    for base in ["test_data", "../test_data"] {
        let path = std::path::Path::new(base).join("reference").join(name);
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(WasmMf4File::new(bytes).expect("open corpus file"));
        }
    }
    None
}

fn require_corpus(name: &str) -> WasmMf4File {
    corpus(name).unwrap_or_else(|| {
        panic!(
            "corpus file {name} must be fetched (scripts/fetch_reference_files.sh) — \
             a skipped test is not a passed test"
        )
    })
}

fn labels_of(payload: &str) -> Vec<Option<String>> {
    let JsonVal::Obj(fields) = parse_json(payload).expect("valid signal_text json") else {
        panic!("signal_text is an object")
    };
    fields
        .into_iter()
        .find_map(|(k, v)| (k == "labels").then_some(v))
        .map(|labels| match labels {
            JsonVal::Array(items) => items
                .into_iter()
                .map(|item| match item {
                    JsonVal::Str(s) => Some(s),
                    JsonVal::Null => None,
                    other => panic!("a label is a string or null, got {other:?}"),
                })
                .collect(),
            other => panic!("labels is an array, got {other:?}"),
        })
        .expect("labels field")
}

fn field_of(payload: &str, key: &str) -> JsonVal {
    let JsonVal::Obj(fields) = parse_json(payload).expect("valid json") else {
        panic!("payload is an object")
    };
    field_in(&fields, key)
}

/// A cloned field value from an object's field list (owned, so assertions can
/// compare `JsonVal`s without fighting reference auto-clone).
fn field_in(fields: &[(String, JsonVal)], key: &str) -> JsonVal {
    fields
        .iter()
        .find_map(|(k, v)| (k == key).then(|| (*v).clone()))
        .unwrap_or_else(|| panic!("no {key} in {fields:?}"))
}

// ---------------------------------------------------------- label run windows

#[test]
fn label_runs_keep_the_first_sample_every_change_and_the_last() {
    // A run's irreducible content is its state changes: the window's first
    // sample, every label change, and the window's last sample (which closes
    // the final run at a real time so a band view reaches the right edge).
    let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let gear = ["N", "1", "2", "3", "2", "1", "N", "N", "N", "N"];
    let labels: Vec<Option<&str>> = gear.iter().map(|&g| Some(g)).collect();
    let (ts, ls, truncated) =
        decimate_label_runs(&times, &labels, f64::NEG_INFINITY, f64::INFINITY, 100);
    assert!(!truncated);
    // Change points 0–6, plus the window's last sample (t=9) closing the
    // final N run at a real time.
    assert_eq!(ts, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 9.0]);
    let flat: Vec<Option<String>> = ls.into_iter().collect();
    assert_eq!(
        flat,
        vec![
            Some("N".into()),
            Some("1".into()),
            Some("2".into()),
            Some("3".into()),
            Some("2".into()),
            Some("1".into()),
            Some("N".into()),
            Some("N".into()),
        ]
    );
}

#[test]
fn label_runs_window_edges_are_inclusive_like_the_numeric_window() {
    // Same contract as decimate_window: [t0, t1] covers both endpoints, so a
    // cursor region's bands agree with its numeric statistics.
    let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let labels: Vec<Option<&str>> = (0..10)
        .map(|i| Some(if i < 5 { "low" } else { "high" }))
        .collect();
    let (ts, ls, _) = decimate_label_runs(&times, &labels, 3.0, 6.0, 100);
    // [3,"low"] opens the window, [5,"high"] is the change, and the window's
    // last sample (6) closes the final run even though its label repeats.
    assert_eq!(ts, vec![3.0, 5.0, 6.0]);
    let flat: Vec<Option<String>> = ls.into_iter().collect();
    assert_eq!(
        flat,
        vec![Some("low".into()), Some("high".into()), Some("high".into())]
    );
}

#[test]
fn label_runs_survive_empty_windows_and_shaped_inputs() {
    let times = [0.0, 1.0, 2.0];
    let labels = [Some("a"), Some("b"), Some("a")];
    // Empty, reversed, zero budget, mismatched lengths: nothing, no panic —
    // the same guards decimate_window applies to a hostile master.
    for (t0, t1, budget) in [(3.0, 1.0, 10), (0.0, 2.0, 0)] {
        let (ts, ls, truncated) = decimate_label_runs(&times, &labels, t0, t1, budget);
        assert!(ts.is_empty() && ls.is_empty() && !truncated);
    }
    let (ts, ls, _) = decimate_label_runs(&times, &labels[..2], 0.0, 2.0, 10);
    assert!(ts.is_empty() && ls.is_empty());
    // An empty series under infinite bounds also yields nothing.
    let (ts, ls, _) = decimate_label_runs(&[], &[], f64::NEG_INFINITY, f64::INFINITY, 10);
    assert!(ts.is_empty() && ls.is_empty());
}

#[test]
fn label_runs_stop_at_the_hard_cap_and_report_truncation() {
    // A label changing on every sample defeats run collapsing; the 1.5×
    // hard cap stops the output and `truncated` says so, exactly like the
    // numeric path's degenerate-column guard.
    let times: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let labels: Vec<Option<&str>> = (0..100)
        .map(|i| Some(if i % 2 == 0 { "a" } else { "b" }))
        .collect();
    let (ts, ls, truncated) = decimate_label_runs(&times, &labels, 0.0, 99.0, 10);
    assert!(truncated, "an every-sample change must report truncation");
    assert_eq!(ts.len(), 15, "hard cap = max_points * 1.5");
    assert_eq!(ls.len(), ts.len());
    assert!(!ls.iter().any(|l| l.is_none()));
}

#[test]
fn invalid_labels_pass_through_as_null_not_silently_dropped() {
    // A None label (invalidation fold) is a state the drawing side must gap,
    // so it is emitted like any label change rather than collapsed away.
    let times = [0.0, 1.0, 2.0, 3.0];
    let labels = [Some("a"), None, None, Some("b")];
    let (ts, ls, _) = decimate_label_runs(&times, &labels, 0.0, 3.0, 100);
    assert_eq!(ts, vec![0.0, 1.0, 3.0]);
    let flat: Vec<Option<String>> = ls.into_iter().collect();
    assert_eq!(flat, vec![Some("a".into()), None, Some("b".into())]);
}

// ---------------------------------------------------------- label statistics

#[test]
fn label_stats_distribute_seconds_by_run_length() {
    // Each sample holds its label until the next one: sample i contributes
    // t[i+1] - t[i], the window's last sample contributes nothing (its end
    // lies outside the window). A(0..2) holds 2 s, B(2..5) holds 3 s, the A
    // sample at 5 is the window's last and holds no time — but counts.
    let times = [0.0, 2.0, 5.0];
    let labels = [Some("A"), Some("B"), Some("A")];
    let json = window_label_stats_json(&times, &labels, 0.0, 5.0);
    let JsonVal::Obj(fields) = parse_json(&json).expect("valid stats json") else {
        panic!("stats is an object")
    };
    assert_eq!(field_in(&fields, "count"), JsonVal::Number(3.0));
    assert_eq!(field_in(&fields, "invalid"), JsonVal::Number(0.0));
    let JsonVal::Array(entries) = field_in(&fields, "labels") else {
        panic!("labels is an array")
    };
    let as_tuple = |e: &JsonVal| match e {
        JsonVal::Obj(f) => (
            field_in(f, "label"),
            field_in(f, "samples"),
            field_in(f, "seconds"),
        ),
        other => panic!("label entry is an object: {other:?}"),
    };
    let parsed: Vec<_> = entries.iter().map(as_tuple).collect();
    // Most-active first: B held 3 s over 1 sample, A held 2 s over 2 samples.
    assert_eq!(
        parsed,
        vec![
            (
                JsonVal::Str("B".into()),
                JsonVal::Number(1.0),
                JsonVal::Number(3.0)
            ),
            (
                JsonVal::Str("A".into()),
                JsonVal::Number(2.0),
                JsonVal::Number(2.0)
            ),
        ]
    );
}

#[test]
fn label_stats_count_invalid_and_clamp_infinite_bounds() {
    // None labels count in `invalid` and nowhere else; infinite bounds clamp
    // to the extent so a viewer may bootstrap with (-Inf, Inf).
    let times = [0.0, 1.0, 2.0];
    let labels = [Some("a"), None, Some("a")];
    let json = window_label_stats_json(&times, &labels, f64::NEG_INFINITY, f64::INFINITY);
    assert!(json.contains("\"count\":2"));
    assert!(json.contains("\"invalid\":1"));
    // The echoed window is the clamp, not the infinities.
    assert!(json.contains("\"t0\":0"));
    assert!(json.contains("\"t1\":2"));
    // A reversed window covers nothing, validly.
    let json = window_label_stats_json(&times, &labels, 2.0, 1.0);
    assert!(json.contains("\"count\":0"));
}

// ------------------------------------------------------------------ label CSV

#[test]
fn labels_csv_quotes_and_empties_like_the_numeric_csv() {
    // RFC 4180: a label with a comma is quoted; an invalid sample (None) is
    // an empty field, matching series_csv's rule for NaN.
    let times = [0.0, 1.0, 2.0];
    let labels = [Some("on, fully"), None, Some("say \"hi\"")];
    let csv = labels_csv(&times, &labels, "state");
    assert_eq!(
        csv,
        "timestamp,state\n0,\"on, fully\"\n1,\n2,\"say \"\"hi\"\"\"\n"
    );
}

// ------------------------------------------------------- corpus-backed checks

#[test]
fn the_vector_text_corpus_reports_text_kinds() {
    // Kind metadata comes from the channel list so a viewer never has to
    // decode to find out — the master must stay "f64" while the data channel
    // is "text" in every flavour of text conversion the corpus carries.
    for name in [
        "Vector_Value2TextConversion.mf4",
        "Vector_ValueRange2TextConversion.mf4",
        "Vector_Text2TextConversion.mf4",
        "Vector_FixedLengthStringUTF8.mf4",
        "Vector_FixedLengthStringUTF16_LE.mf4",
        "Vector_FixedLengthStringSBC.mf4",
    ] {
        let file = require_corpus(name);
        assert_eq!(
            file.channel_kind("Data channel").expect("kind"),
            "text",
            "{name}: the data channel decodes as text"
        );
        assert_eq!(
            file.channel_kind("Time channel").expect("kind"),
            "f64",
            "{name}: the master stays numeric"
        );
        // signal_text refuses the numeric channel rather than lying with NaNs.
        let mut file = file;
        assert!(
            file.signal_text("Time channel", 0.0, 9.0, 100).is_err(),
            "{name}: signal_text on the master must be an error"
        );
    }
}

#[test]
fn a_byte_array_channel_reports_bytes_not_text() {
    // The negative of the text kind: opaque bytes must not be offered to the
    // label path (and their scalar view is all-NaN, never silently shipped).
    let Some(file) = corpus("Vector_ByteArrayFixedLength.mf4") else {
        return; // corpus not fetched
    };
    let kind = file.channel_kind("Data channel").expect("kind");
    assert_eq!(kind, "bytes");
}

#[test]
fn signal_text_returns_the_value2text_labels() {
    // Pinned against asammdf: 10 samples at 1 s, gears 1–5 with "No match"
    // before and after. Run collapsing keeps 7 of the 10 samples.
    let mut file = require_corpus("Vector_Value2TextConversion.mf4");
    let json = file
        .signal_text("Data channel", f64::NEG_INFINITY, f64::INFINITY, 100)
        .expect("signal_text");
    assert!(json.contains("\"kind\":\"text\""));
    assert!(json.contains("\"truncated\":false"));
    // The change points, plus the series' last sample (t=9) closing the
    // final "No match" run.
    assert_eq!(
        labels_of(&json),
        vec![
            Some("No match".into()),
            Some("first gear".into()),
            Some("second gear".into()),
            Some("third gear".into()),
            Some("fourth gear".into()),
            Some("fifth gear".into()),
            Some("No match".into()),
            Some("No match".into()),
        ]
    );
}

#[test]
fn signal_text_sub_window_keeps_inclusive_edges() {
    // Window [3, 6] of the value-range file covers samples 3..=6:
    // low(3), low(4), medium(5), medium(6) → runs [3,"low"], [5,"medium"],
    // plus the window's last sample closing the final run at t=6.
    let mut file = require_corpus("Vector_ValueRange2TextConversion.mf4");
    let json = file
        .signal_text("Data channel", 3.0, 6.0, 100)
        .expect("signal_text");
    assert_eq!(
        labels_of(&json),
        vec![
            Some("low".into()),
            Some("medium".into()),
            Some("medium".into())
        ]
    );
}

#[test]
fn signal_text_round_trips_utf8_labels() {
    // Text2Text translates to German; "Fünf" is multi-byte and must survive
    // the JSON escaping byte-for-byte.
    let mut file = require_corpus("Vector_Text2TextConversion.mf4");
    let json = file
        .signal_text("Data channel", f64::NEG_INFINITY, f64::INFINITY, 100)
        .expect("signal_text");
    assert!(json.contains("Fünf"), "utf-8 label survives: {json}");
}

#[test]
fn signal_stats_on_a_text_channel_is_a_label_distribution() {
    // Pinned against asammdf's samples: "No match" holds samples 0 and 6..9.
    // Sample 0 contributes [0,1) = 1 s; samples 6, 7, 8 contribute 1 s each;
    // sample 9 is the window's last and contributes nothing → 4 s over
    // 5 samples. Each gear holds exactly 1 s over 1 sample.
    let mut file = require_corpus("Vector_Value2TextConversion.mf4");
    let json = file
        .signal_stats("Data channel", f64::NEG_INFINITY, f64::INFINITY)
        .expect("stats");
    let JsonVal::Obj(fields) = parse_json(&json).expect("valid stats json") else {
        panic!("stats is an object")
    };
    assert_eq!(field_in(&fields, "count"), JsonVal::Number(10.0));
    let JsonVal::Array(entries) = field_in(&fields, "labels") else {
        panic!("labels is an array")
    };
    let no_match = entries.iter().find_map(|e| match e {
        JsonVal::Obj(f) if field_in(f, "label") == JsonVal::Str("No match".into()) => {
            Some(f.clone())
        }
        _ => None,
    });
    let Some(no_match) = no_match else {
        panic!("No match entry missing from distribution: {entries:?}")
    };
    assert_eq!(field_in(&no_match, "samples"), JsonVal::Number(5.0));
    assert_eq!(field_in(&no_match, "seconds"), JsonVal::Number(4.0));
    assert_eq!(entries.len(), 6, "No match plus five gears");
}

#[test]
fn signal_csv_exports_text_labels() {
    // The CSV of a text channel carries the labels — an empty value column
    // would be the CSV form of the NaN this path used to ship.
    let mut file = require_corpus("Vector_Value2TextConversion.mf4");
    let csv = file
        .signal_csv("Data channel", f64::NEG_INFINITY, f64::INFINITY)
        .expect("csv");
    let lines: Vec<&str> = csv.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines.len(), 11, "header plus 10 samples");
    assert_eq!(lines[0], "timestamp,Data channel");
    assert_eq!(lines[1], "0,No match");
    assert_eq!(lines[2], "1,first gear");
    assert_eq!(lines[10], "9,No match");
}

#[test]
fn a_text_channel_refuses_the_array_path_and_versa() {
    // The kind strings are a contract between endpoints: each refuses what
    // another kind's endpoint is for. (The thrown message naming the kind is
    // a wasm-runtime property; natively js_err carries no message, and a
    // JsValue's Debug is itself off-limits off-wasm — assert error-ness only,
    // like the native-limit test in window.rs.)
    let mut file = require_corpus("Vector_Value2TextConversion.mf4");
    assert!(file
        .signal_element_window("Data channel", 0, 0.0, 9.0, 100)
        .is_err());
    let mut file = require_corpus("Vector_ByteArrayFixedLength.mf4");
    assert!(file.signal_text("Data channel", 0.0, 9.0, 100).is_err());
}
