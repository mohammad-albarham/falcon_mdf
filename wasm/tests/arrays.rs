//! Native tests for the array/bytes view (plan 2.4): element extraction
//! (`element_values` — the pure core of `signal_element_window`), the raw
//! index-range pages the sample table serves from (`raw_page`), and the
//! kind contract on the Vector corpus files.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf_wasm::{element_values, fold_validity, WasmMf4File};

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

// ------------------------------------------------ dynamic array & validity

const HEADER: usize = 24;

fn synth_block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
    let total = HEADER + links.len() * 8 + data.len();
    let mut out = vec![0u8; HEADER];
    out[0..4].copy_from_slice(id);
    out[8..16].copy_from_slice(&(total as u64).to_le_bytes());
    out[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());
    for link in links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

fn synth_tx(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    while !data.len().is_multiple_of(8) {
        data.push(0);
    }
    synth_block(b"##TX", &[], &data)
}

#[allow(clippy::too_many_arguments)]
fn synth_cn(
    next: u64,
    composition: u64,
    name: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
    flags: u32,
    inval_bit_pos: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    d[12..16].copy_from_slice(&flags.to_le_bytes());
    d[16..20].copy_from_slice(&inval_bit_pos.to_le_bytes());
    synth_block(b"##CN", &[next, composition, name, 0, 0, 0, 0, 0], &d)
}

fn dynamic_array_file(with_invalidation: bool) -> WasmMf4File {
    let mut bytes = vec![0u8; 64];
    bytes[0..8].copy_from_slice(b"MDF     ");
    bytes[8..16].copy_from_slice(b"4.11    ");
    bytes[16..24].copy_from_slice(b"falcon  ");
    bytes[28..30].copy_from_slice(&411u16.to_le_bytes());

    let push = |buf: &mut Vec<u8>, blk: &[u8]| -> u64 {
        let at = buf.len() as u64;
        buf.extend_from_slice(blk);
        at
    };
    let patch = |buf: &mut Vec<u8>, at: u64, val: u64| {
        let idx = at as usize;
        buf[idx..idx + 8].copy_from_slice(&val.to_le_bytes());
    };

    let mut hd_data = vec![0u8; 32];
    hd_data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
    let hd_at = push(&mut bytes, &synth_block(b"##HD", &[0; 6], &hd_data));
    assert_eq!(hd_at, 64);

    let count_name = push(&mut bytes, &synth_tx("Count"));
    let values_name = push(&mut bytes, &synth_tx("Values"));
    let template = push(&mut bytes, &synth_cn(0, 0, 0, 0, 4, 0, 64, 0, 0));

    let mut ca_data = Vec::new();
    ca_data.push(0u8); // ca_type = Array
    ca_data.push(0u8); // ca_storage = CN template
    ca_data.extend_from_slice(&1u16.to_le_bytes()); // 1D
    ca_data.extend_from_slice(&1u32.to_le_bytes()); // dynamic size flag (bit 0)
    ca_data.extend_from_slice(&8i32.to_le_bytes()); // element stride = 8 bytes
    ca_data.extend_from_slice(&0u32.to_le_bytes());
    ca_data.extend_from_slice(&3u64.to_le_bytes()); // max 3 elements
    let array = push(
        &mut bytes,
        &synth_block(b"##CA", &[template, 0, 0, 0], &ca_data),
    );

    let cn_flags = if with_invalidation { 0x02 } else { 0 };
    let values_channel = push(
        &mut bytes,
        &synth_cn(0, array, values_name, 0, 4, 1, 64, cn_flags, 0),
    );
    let count_channel = push(
        &mut bytes,
        &synth_cn(values_channel, 0, count_name, 0, 0, 0, 8, 0, 0),
    );

    let mut cg_data = vec![0u8; 32];
    cg_data[8..16].copy_from_slice(&2u64.to_le_bytes()); // 2 samples
    cg_data[24..28].copy_from_slice(&25u32.to_le_bytes()); // 25 data bytes
    if with_invalidation {
        cg_data[28..32].copy_from_slice(&1u32.to_le_bytes()); // 1 invalidation byte
    }
    let group = push(
        &mut bytes,
        &synth_block(b"##CG", &[0, count_channel, 0, 0, 0, 0], &cg_data),
    );

    let mut records = Vec::new();
    // Sample 0: 2 of 3 elements used ([1.0, 2.0]), valid
    records.push(2u8);
    records.extend_from_slice(&1.0f64.to_le_bytes());
    records.extend_from_slice(&2.0f64.to_le_bytes());
    records.extend_from_slice(&99.0f64.to_le_bytes());
    if with_invalidation {
        records.push(0x00); // sample 0 valid
    }
    // Sample 1: 3 of 3 elements used ([4.0, 5.0, 6.0]), invalid when requested
    records.push(3u8);
    records.extend_from_slice(&4.0f64.to_le_bytes());
    records.extend_from_slice(&5.0f64.to_le_bytes());
    records.extend_from_slice(&6.0f64.to_le_bytes());
    if with_invalidation {
        records.push(0x01); // sample 1 invalid (bit 0 = 1)
    }

    let data = push(&mut bytes, &synth_block(b"##DT", &[], &records));
    let group_block = push(
        &mut bytes,
        &synth_block(b"##DG", &[0, group, data, 0], &[0u8; 8]),
    );
    patch(&mut bytes, 64 + HEADER as u64, group_block);

    let triple_at = array + HEADER as u64 + 8;
    patch(&mut bytes, triple_at, group_block);
    patch(&mut bytes, triple_at + 8, group);
    patch(&mut bytes, triple_at + 16, count_channel);

    WasmMf4File::new(bytes).expect("open dynamic array file")
}

#[test]
fn raw_page_formats_dynamic_length_array_with_valid_json() {
    let mut file = dynamic_array_file(false);
    let page = file
        .raw_page("Values", 0, 2)
        .expect("raw_page on ArrayVarLen");
    let JsonVal::Obj(fields) =
        parse_json(&page).expect("ArrayVarLen raw_page must emit valid JSON")
    else {
        panic!("page is an object")
    };
    assert_eq!(page_field(&fields, "kind"), JsonVal::Str("array".into()));
    assert_eq!(page_field(&fields, "count"), JsonVal::Number(2.0));
    assert_eq!(
        page_field(&fields, "elems"),
        JsonVal::Array(vec![
            JsonVal::Number(1.0),
            JsonVal::Number(2.0),
            JsonVal::Number(4.0),
            JsonVal::Number(5.0),
            JsonVal::Number(6.0),
        ])
    );
    assert_eq!(
        page_field(&fields, "starts"),
        JsonVal::Array(vec![
            JsonVal::Number(0.0),
            JsonVal::Number(2.0),
            JsonVal::Number(5.0),
        ])
    );

    // Sub-page starting at sample 1 rebases page-local starts to 0.
    let sub = file
        .raw_page("Values", 1, usize::MAX)
        .expect("sub-page with saturating count");
    let JsonVal::Obj(sub_fields) = parse_json(&sub).expect("sub-page valid JSON") else {
        panic!("sub-page is an object")
    };
    assert_eq!(
        page_field(&sub_fields, "starts"),
        JsonVal::Array(vec![JsonVal::Number(0.0), JsonVal::Number(3.0)])
    );
}

#[test]
fn fold_validity_invalidates_all_elements_of_fixed_and_varlen_arrays() {
    // Fixed-shape array: 3 samples × 2 elements = 6 values, validity length 3.
    let mut fixed = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    fold_validity(&mut fixed, Some(&[true, false, true]), None);
    assert_same(fixed.to_vec(), &[1.0, 2.0, f64::NAN, f64::NAN, 5.0, 6.0]);

    // Variable-length array: sample 0 has 2 elements, sample 1 has 3 elements.
    let mut varlen = [1.0, 2.0, 4.0, 5.0, 6.0];
    let starts = [0usize, 2, 5];
    fold_validity(&mut varlen, Some(&[true, false]), Some(&starts));
    assert_same(varlen.to_vec(), &[1.0, 2.0, f64::NAN, f64::NAN, f64::NAN]);

    // End-to-end through WasmMf4File::raw_page and WasmMf4File::signal on an
    // ArrayVarLen channel with sample 1 marked invalid.
    let mut file = dynamic_array_file(true);
    let page = file.raw_page("Values", 0, 2).expect("raw_page");
    let JsonVal::Obj(fields) = parse_json(&page).expect("valid json") else {
        panic!("page is an object")
    };
    assert_eq!(
        page_field(&fields, "elems"),
        JsonVal::Array(vec![
            JsonVal::Number(1.0),
            JsonVal::Number(2.0),
            JsonVal::Null,
            JsonVal::Null,
            JsonVal::Null,
        ])
    );

    let sig = file.signal("Values").expect("signal");
    let JsonVal::Obj(sig_fields) = parse_json(&sig).expect("signal json") else {
        panic!("signal is an object")
    };
    assert_eq!(
        page_field(&sig_fields, "values"),
        JsonVal::Array(vec![
            JsonVal::Number(1.0),
            JsonVal::Number(2.0),
            JsonVal::Null,
            JsonVal::Null,
            JsonVal::Null,
        ])
    );
}
