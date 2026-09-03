//! Native tests for the array/bytes view (plan 2.4): element extraction
//! (`element_values` — the pure core of `signal_element_window`), the raw
//! index-range pages the sample table serves from (`raw_page`), and the
//! kind contract on the Vector corpus files.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf_wasm::{element_values, WasmMf4File};

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
        panic!("corpus file {name} must be fetched — a skipped test is not a passed test")
    })
}

// --------------------------------------------------------- element extraction

/// Element-wise equality that treats NaN == NaN: gaps are expected values
/// here, and `assert_eq!` on floats would fail against itself.
fn assert_same(actual: Vec<f64>, expected: &[f64]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "length mismatch: {actual:?} vs {expected:?}"
    );
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert!(
            a == e || (a.is_nan() && e.is_nan()),
            "sample {i}: {actual:?} vs {expected:?}"
        );
    }
}

#[test]
fn fixed_shape_elements_address_by_eps() {
    // Element j of sample i lives at i * eps + j; a sample cannot lack an
    // element in the fixed case.
    let values = [10.0, 11.0, 20.0, 21.0, 30.0, 31.0];
    assert_same(element_values(&values, None, 2, 0), &[10.0, 20.0, 30.0]);
    assert_same(element_values(&values, None, 2, 1), &[11.0, 21.0, 31.0]);
}

#[test]
fn dynamic_shape_elements_fall_back_to_nan_for_short_samples() {
    // starts[i]..starts[i+1] is sample i; a sample shorter than the asked
    // element has no such value — NaN, a gap, never a neighbouring sample's
    // element and never a zero that would read as a measurement.
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    let starts = [0usize, 2, 2, 5]; // sample 1 is empty
    assert_same(
        element_values(&values, Some(&starts), 0, 0),
        &[1.0, f64::NAN, 3.0],
    );
    assert_same(
        element_values(&values, Some(&starts), 0, 2),
        &[f64::NAN, f64::NAN, 5.0],
    );
    // An element index past every sample is all gaps, not an error: the
    // selector UI can be driven by a stale count without a panic.
    assert!(element_values(&values, Some(&starts), 0, 99)
        .iter()
        .all(|v| v.is_nan()));
    // An inconsistent starts vector (last entry off the end) must not panic:
    // the bounds saturate and whatever flat value is addressable comes back.
    let bad = [0usize, 99];
    assert_same(element_values(&values, Some(&bad), 0, 0), &[1.0]);
}

// --------------------------------------------------------------- raw pages

/// The array channel the MeasurementArrays fixture plots with, plus its
/// element count — `Curve1` is a declared-shape `[8]` curve. The file has no
/// "Data channel": its arrays are named (Curve1, map5_82_uc, …).
const ARRAY_CHANNEL: &str = "Curve1";

fn page_field(fields: &[(String, JsonVal)], key: &str) -> JsonVal {
    fields
        .iter()
        .find_map(|(k, v)| (k == key).then(|| v.clone()))
        .unwrap_or_else(|| panic!("no {key} in page"))
}

#[test]
fn raw_page_slices_a_fixed_shape_array_with_eps() {
    // Vector's measurement arrays: the file defines the shape, so the page's
    // eps must equal the shape's product and elems must be the flat values.
    let mut file = require_corpus("Vector_MeasurementArrays.mf4");
    assert_eq!(file.channel_kind(ARRAY_CHANNEL).expect("kind"), "array");
    let shape = file.array_shape(ARRAY_CHANNEL).expect("shape");
    let JsonVal::Array(dims) = parse_json(&shape).expect("dims json") else {
        panic!("shape is an array")
    };
    let eps: f64 = dims
        .iter()
        .map(|d| match d {
            JsonVal::Number(n) => *n,
            other => panic!("dim is a number, got {other:?}"),
        })
        .product();
    assert!(eps > 1.0, "a real array shape, got {dims:?}");

    let page = file.raw_page(ARRAY_CHANNEL, 0, 2).expect("page");
    let JsonVal::Obj(fields) = parse_json(&page).expect("page json") else {
        panic!("page is an object")
    };
    assert_eq!(page_field(&fields, "kind"), JsonVal::Str("array".into()));
    assert_eq!(page_field(&fields, "eps"), JsonVal::Number(eps));
    let JsonVal::Number(total) = page_field(&fields, "total") else {
        panic!("total is a number")
    };
    assert_eq!(page_field(&fields, "count"), JsonVal::Number(2.0));
    let JsonVal::Array(elems) = page_field(&fields, "elems") else {
        panic!("elems is an array")
    };
    assert_eq!(
        elems.len() as f64,
        eps * 2.0,
        "2 rows of eps elements (total {total})"
    );
}

#[test]
fn raw_page_clamps_ranges_past_the_end() {
    // A start past the end clamps to an empty page at the total; a count
    // past the end truncates. (Page-local starts arithmetic on the dynamic
    // path is covered by the pure element_values tests.)
    let mut file = require_corpus("Vector_MeasurementArrays.mf4");
    let page = file.raw_page(ARRAY_CHANNEL, 99, 10).expect("page");
    let JsonVal::Obj(fields) = parse_json(&page).expect("page json") else {
        panic!("page is an object")
    };
    let (JsonVal::Number(start), JsonVal::Number(total), JsonVal::Number(count)) = (
        page_field(&fields, "start"),
        page_field(&fields, "total"),
        page_field(&fields, "count"),
    ) else {
        panic!("page header fields are numbers")
    };
    assert_eq!(start, total, "start clamps to the total");
    assert_eq!(count, 0.0, "a clamped page is empty");

    let page = file.raw_page(ARRAY_CHANNEL, 0, 99).expect("page");
    let JsonVal::Obj(fields) = parse_json(&page).expect("page json") else {
        panic!("page is an object")
    };
    assert_eq!(
        page_field(&fields, "count"),
        page_field(&fields, "total"),
        "count truncates at total"
    );
}

#[test]
fn raw_page_hexes_bytes_and_refuses_scalars() {
    // Bytes come back as per-sample hex; a scalar channel is refused so each
    // kind keeps exactly one tabling path.
    let mut file = require_corpus("Vector_ByteArrayFixedLength.mf4");
    let page = file.raw_page("Data channel", 0, 3).expect("page");
    let JsonVal::Obj(fields) = parse_json(&page).expect("page json") else {
        panic!("page is an object")
    };
    let field = |k: &str| {
        fields
            .iter()
            .find_map(|(key, v)| (key == k).then_some(v.clone()))
            .unwrap_or_else(|| panic!("no {k} in page"))
    };
    assert_eq!(field("kind"), JsonVal::Str("bytes".into()));
    let JsonVal::Array(hex) = field("hex") else {
        panic!("hex is an array")
    };
    assert_eq!(hex.len(), 3);
    for h in &hex {
        let JsonVal::Str(s) = h else {
            panic!("hex entries are strings, got {h:?}")
        };
        assert!(
            !s.is_empty(),
            "the file's samples are not zero-width: {s:?}"
        );
    }
    assert!(file.raw_page("Time channel", 0, 3).is_err());
}

// ------------------------------------------------------- element windows

#[test]
fn signal_element_window_plots_one_element_spike_safe() {
    // Element 0 of the measurement array, windowed like any scalar: the
    // reply carries the selectable element count so the UI can offer
    // 0..elements without a second call.
    let mut file = require_corpus("Vector_MeasurementArrays.mf4");
    // signal_element_window builds JS objects; off-wasm it is the documented
    // native-limit error (see window.rs) — the pure core is tested above and
    // the browser pass covers the live path.
    assert!(file
        .signal_element_window(ARRAY_CHANNEL, 0, 0.0, 9.0, 100)
        .is_err());
    // Metadata still answers natively: shape and kind agree with the page.
    let shape = file.array_shape(ARRAY_CHANNEL).expect("shape");
    assert!(shape.starts_with('['));
}
