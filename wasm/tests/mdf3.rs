//! Native tests for MDF 3.x through the same binding (plan 3.1): the
//! signature sniff routes a v3 file to the v3 reader, and every endpoint the
//! viewer uses — names, kinds, the scalar window's input, text channels,
//! stats, CSV — answers for it.
//!
//! Fixtures are written by asammdf (the reference implementation) into a
//! temp dir, exactly like the core crate's mdf3 conformance tests; without a
//! `.venv` the tests skip, which is why fetching the environment first is a
//! repo rule (`a skipped test is not a passed test`).

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf_wasm::WasmMf4File;

/// Writes a small MDF 3.30 file with asammdf: two numeric channels on one
/// master and one text channel, the flavours the viewer plots. Returns the
/// file's bytes, or `None` without a usable `.venv`.
///
/// The fixture lands in the OS temp dir under this test's exclusive name —
/// no temp-file dependency for the wasm crate (the dependency rule), and no
/// parallel-run collisions.
fn v3_fixture() -> Option<Vec<u8>> {
    let python = ["../.venv/bin/python", ".venv/bin/python"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())?;
    let dir = std::env::temp_dir().join(format!("falcon_wasm_mdf3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("gen_v330.mdf");
    let script = format!(
        r#"
import numpy as np
from asammdf import MDF, Signal

t = np.arange(0.0, 1.0, 0.1)
sigs = [
    Signal(samples=(t * 3.0).astype(np.float64), timestamps=t, name="Speed", unit="km/h"),
    Signal(samples=(t * 100).astype(np.uint8), timestamps=t, name="Gear", unit=""),
    Signal(samples=np.array([b"off"] * 5 + [b"on"] * 5), timestamps=t, name="State", unit=""),
]
m = MDF(version="3.30")
m.append(sigs)
m.save(r"{path}", overwrite=True)
m.close()
"#,
        path = path.display()
    );
    let out = std::process::Command::new(python)
        .arg("-c")
        .arg(&script)
        .output()
        .expect("running asammdf should succeed");
    assert!(
        out.status.success(),
        "asammdf failed to write the v3 fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&path).expect("read the written fixture");
    // Keep a copy where the browser pass can deep-link it (?file=), the same
    // end-to-end path the demo ships to users.
    let _ = std::fs::copy(
        &path,
        std::path::Path::new("../test_data/generated/gen_v330.mdf"),
    );
    Some(bytes)
}

fn require_v3() -> WasmMf4File {
    let Some(bytes) = v3_fixture() else {
        panic!("a .venv with asammdf is required — a skipped test is not a passed test")
    };
    WasmMf4File::new(bytes).expect("the sniffed v3 file should open")
}

fn names_of(file: &WasmMf4File) -> Vec<String> {
    match parse_json(&file.channel_names().expect("names")).expect("names json") {
        JsonVal::Array(items) => items
            .into_iter()
            .map(|v| match v {
                JsonVal::Str(s) => s,
                other => panic!("names are strings, got {other:?}"),
            })
            .collect(),
        other => panic!("names is an array, got {other:?}"),
    }
}

#[test]
fn a_v3_file_opens_and_reports_its_channels() {
    let file = require_v3();
    let names = names_of(&file);
    for expected in ["Speed", "Gear", "State"] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} in {names:?}"
        );
    }
    assert!(file.channel_count() >= 4, "three channels plus the master");
}

#[test]
fn v3_kinds_are_derived_from_metadata() {
    let file = require_v3();
    assert_eq!(file.channel_kind("Speed").expect("kind"), "f64");
    assert_eq!(file.channel_kind("Gear").expect("kind"), "f64");
    assert_eq!(file.channel_kind("State").expect("kind"), "text");
    // asammdf names the v3 master "time"; it is numeric like every master.
    assert_eq!(file.channel_kind("time").expect("kind"), "f64");
    // Asking for a missing channel is an error, not an empty kind.
    assert!(file.channel_kind("Nope").is_err());
}

#[test]
fn v3_info_carries_the_version() {
    let file = require_v3();
    let json = file.info().expect("info");
    assert!(json.contains("3.30"), "version text in info: {json}");
    assert!(json.contains("\"channel_count\""));
}

#[test]
fn v3_scalar_signal_carries_the_expected_samples() {
    // Pinned against the generator: Speed is t * 3.0 over t = 0..1 step .1.
    let file = require_v3();
    let json = file.signal("Speed").expect("signal");
    let JsonVal::Obj(fields) = parse_json(&json).expect("signal json") else {
        panic!("signal is an object")
    };
    let field = |k: &str| {
        fields
            .iter()
            .find_map(|(key, v)| (key == k).then_some(v.clone()))
            .unwrap_or_else(|| panic!("no {k}"))
    };
    assert_eq!(field("unit"), JsonVal::Str("km/h".into()));
    let JsonVal::Array(values) = field("values") else {
        panic!("values is an array")
    };
    assert_eq!(values.len(), 10);
    match &values[3] {
        JsonVal::Number(v) => assert!((v - 0.9).abs() < 1e-9, "t=0.3 → 0.9 km/h, got {v}"),
        other => panic!("a sample is a number, got {other:?}"),
    }
}

#[test]
fn v3_text_channel_answers_signal_text() {
    let mut file = require_v3();
    let json = file
        .signal_text("State", f64::NEG_INFINITY, f64::INFINITY, 100)
        .expect("signal_text");
    assert!(json.contains("\"kind\":\"text\""));
    assert!(json.contains("\"off\""), "first state in {json}");
    assert!(json.contains("\"on\""), "second state in {json}");
    // And the scalar endpoints refuse what they always refused.
    assert!(file.signal_text("Speed", 0.0, 1.0, 100).is_err());
}

#[test]
fn v3_stats_and_csv_work_like_the_v4_ones() {
    let mut file = require_v3();
    let stats = file
        .signal_stats("Speed", f64::NEG_INFINITY, f64::INFINITY)
        .expect("stats");
    assert!(stats.contains("\"count\":10"), "ten samples: {stats}");
    let csv = file
        .signal_csv("Speed", f64::NEG_INFINITY, f64::INFINITY)
        .expect("csv");
    let lines: Vec<&str> = csv.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines.len(), 11, "header plus ten samples");
    assert_eq!(lines[0], "timestamp,Speed");
}

#[test]
fn v3_search_and_details_work_like_the_v4_ones() {
    let file = require_v3();
    let json = file.search_channels("speed", "contains").expect("search");
    assert!(json.contains("Speed"), "case-insensitive contains: {json}");
    let json = file.search_channels("S????", "wildcard").expect("search");
    assert!(json.contains("Speed"), "wildcard whole-name: {json}");
    let details = file.channel_details("Gear").expect("details");
    assert!(
        details.contains("\"kind\":\"f64\"") && details.contains("\"samples\":10"),
        "details carry kind and sample count: {details}"
    );
}
