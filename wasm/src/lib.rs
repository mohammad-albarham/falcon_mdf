//! WebAssembly bindings for `falcon_mdf` via `wasm-bindgen`.
//!
//! Exposes a reading API over in-memory MF4 files for WebAssembly and JavaScript runtimes:
//!
//! - [`WasmMf4File::new`] reads an MF4 file from raw bytes (e.g. `Uint8Array`).
//! - [`WasmMf4File::channel_names`] lists every channel name in the file as a JSON array of strings.
//! - [`WasmMf4File::channel_count`] returns the total number of channels.
//! - [`WasmMf4File::search_channels`] filters channel names (contains / wildcard / exact)
//!   against the reader's name index, as a JSON array of strings.
//! - [`WasmMf4File::signal`] returns a channel's samples as a JSON object with timestamps and values.
//! - [`WasmMf4File::info`] returns file metadata (version, start time, group and channel counts) as a JSON object.
//! - [`WasmMf4File::channels`] returns every channel's metadata (name, unit, group, description) in one JSON call.
//! - [`WasmMf4File::signal_arrays`] returns a channel's samples as `Float64Array`s (`NaN` stays `NaN`).
//! - [`WasmMf4File::signal_window`] returns a time window of a channel, decimated in Rust to a
//!   point budget so the browser never receives more points than it draws.
//! - [`WasmMf4File::signal_stats`] returns statistics over a time window of a channel (count,
//!   invalid, min, max, mean, first/last) as JSON, so a cursor region costs one round trip.
//! - [`WasmMf4File::signal_csv`] formats a time window of one channel as CSV, in Rust.
//! - [`WasmMf4File::channel_kind`] reports how a channel's samples decode — `"f64"`, `"text"`,
//!   `"bytes"` or `"array"` — and [`WasmMf4File::channels`] carries the same field per entry,
//!   so a viewer can mark unplotable channels without decoding them.
//! - [`WasmMf4File::signal_text`] returns a text channel's labels (value↔text conversions
//!   included), run-collapsed and budgeted like [`WasmMf4File::signal_window`].
//! - [`WasmMf4File::signal_element_window`] returns one element of an array channel as the same
//!   shape [`WasmMf4File::signal_window`] gives a scalar channel; [`WasmMf4File::array_shape`]
//!   names the shape those elements come from.
//! - [`WasmMf4File::raw_page`] serves an index-range page of an array or bytes channel's raw
//!   payloads (flat elements / hex) — the sample table's data path for kinds one numeric
//!   column cannot hold.
//! - [`WasmMf4File::structure`] returns the file's internal outline (block count, history,
//!   attachments, events, channel hierarchy, data groups → channel groups → channels) as one
//!   JSON document, metadata only.
//!
//! A wasm panic would kill the whole module for every caller, so nothing here may
//! panic: no `unwrap`/`expect`, no panicking indexing, and every error crosses
//! into JS as a thrown `Error` via [`js_err`].

use std::cmp::Ordering;
use std::fmt::Write;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

use falcon_mdf::blocks::ChannelType;
use falcon_mdf::candb::CanDatabase;
use falcon_mdf::error::Mf4Error;
use falcon_mdf::io::memory::MemorySource;
use falcon_mdf::mdf3::Mdf3File;
use falcon_mdf::{Channel, Mf4File, SearchMode, SignalValues, ValueKind};

/// Which reader holds the open file. MDF 3 is a different format from MDF 4
/// (not an older spelling of it), so the core gives it a different reader;
/// the sniff in [`WasmMf4File::new`] picks one from the file's signature and
/// everything above this enum is format-agnostic.
// One reader is held per open file, not per sample; boxing would add an
// allocation without reducing the storage occupied by decoded signals.
#[allow(clippy::large_enum_variant)]
enum Inner {
    V4(Mf4File),
    V3(Mdf3File),
}

impl Inner {
    /// Every channel name, sorted and deduplicated — both readers keep the
    /// same contract here.
    fn channel_names(&self) -> Vec<String> {
        match self {
            Inner::V4(f) => f.channel_names().into_iter().map(str::to_string).collect(),
            Inner::V3(f) => f.channel_names().into_iter().map(str::to_string).collect(),
        }
    }

    fn channel_count(&self) -> usize {
        match self {
            Inner::V4(f) => f.channel_count(),
            Inner::V3(f) => f.channel_count(),
        }
    }

    /// How a channel decodes, from metadata alone. MDF 3 has no arrays and
    /// no VLSD: its data-type codes decide between numeric, text (7) and
    /// bytes (8) — the same kinds [`kind_of_channel`] reports for v4.
    fn channel_kind(&self, name: &str) -> Option<&'static str> {
        match self {
            Inner::V4(f) => f.find_channel(name).map(kind_of_channel),
            Inner::V3(f) => {
                for dg in f.data_groups() {
                    for cg in &dg.channel_groups {
                        if let Some(ch) = cg.channels.iter().find(|ch| ch.name == name) {
                            return Some(match ch.data_type {
                                7 => "text",
                                8 => "bytes",
                                _ => "f64",
                            });
                        }
                    }
                }
                None
            }
        }
    }
}

/// Builds `signal()`'s JSON: `{"name", "unit", "timestamps": [...],
/// "values": [...]}`, non-finite floats as `null`.
fn signal_json(
    name: &str,
    unit: &str,
    timestamps: &[f64],
    values: &[f64],
) -> Result<String, JsValue> {
    let mut out = String::new();
    out.push_str("{\"name\":\"");
    escape_json_str_into(name, &mut out);
    out.push_str("\",\"unit\":\"");
    escape_json_str_into(unit, &mut out);
    out.push_str("\",\"timestamps\":[");
    for (i, &t) in timestamps.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_f64(&mut out, t);
    }
    out.push_str("],\"values\":[");
    for (i, &v) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_f64(&mut out, v);
    }
    out.push_str("]}");
    Ok(out)
}

/// Nanoseconds since the epoch as ISO 8601 UTC with milliseconds — the
/// same shape the v4 `start_time().to_iso8601()` emits, so the viewer's
/// `Date.parse` path treats both formats identically. Civil-from-days
/// arithmetic; no calendar dependency.
fn ns_to_iso8601(ns: u64) -> String {
    let secs = (ns / 1_000_000_000) as i64;
    let ms = (ns % 1_000_000_000) / 1_000_000;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days: valid for the full u64 ns range's
    // first milliseconds (1970) through year 2554, far past either format's
    // realistic timestamps.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, mi, sec) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{sec:02}.{ms:03}Z")
}

/// Whole-name wildcard match, the reader's `SearchMode::Wildcard` semantics:
/// `*` any run (including empty), `?` exactly one character, all else
/// literal. Implemented here because the reader's matcher is crate-private.
fn wildcard_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    // Iterative two-pointer with backtracking on the last `*`: no recursion,
    // no quadratic blowup on hostile patterns like `*a*a*a…a`.
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The GPS track's point ceiling. A recording's whole drive path fits a
/// canvas far below this; above it the stride decimation keeps the payload
/// bounded without changing the track's shape.
const TRACK_MAX_POINTS: f64 = 20_000.0;

/// Whether a channel name reads as a latitude coordinate — the GUI panel's
/// heuristic, ported: direct and tokenized matches on the usual names,
/// prefix-stripped vendor spellings, with dynamics (lateral acceleration)
/// and diagnostics disqualified.
pub fn is_latitude_channel_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() || disqualified_position_name(&lower) || lower.contains("lateral") {
        return false;
    }
    if lower == "lat" || lower == "latitude" {
        return true;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.contains(&"latitude") || tokens.contains(&"lat") {
        return true;
    }
    for prefix in [
        "gps",
        "gnss",
        "pos",
        "position",
        "nav",
        "rt",
        "vbox",
        "ins",
        "can_gps",
        "vehicle_gps",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
            if rest == "lat"
                || rest == "latitude"
                || rest.starts_with("lat_")
                || rest.starts_with("latitude_")
                || rest == "latdeg"
                || rest == "latdegrees"
                || rest == "latitudedeg"
                || rest == "latitudedegrees"
            {
                return true;
            }
        }
    }
    false
}

/// Whether a channel name reads as a longitude coordinate.
pub fn is_longitude_channel_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() || disqualified_position_name(&lower) || lower.contains("longitudinal") {
        return false;
    }
    if lower == "lon" || lower == "long" || lower == "longitude" {
        return true;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.contains(&"longitude") || tokens.contains(&"lon") || tokens.contains(&"long") {
        return true;
    }
    for prefix in [
        "gps",
        "gnss",
        "pos",
        "position",
        "nav",
        "rt",
        "vbox",
        "ins",
        "can_gps",
        "vehicle_gps",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
            if rest == "lon"
                || rest == "long"
                || rest == "longitude"
                || rest.starts_with("lon_")
                || rest.starts_with("longitude_")
                || rest == "londeg"
                || rest == "londegrees"
                || rest == "longitudeeg"
                || rest == "longitudedegrees"
            {
                return true;
            }
        }
    }
    false
}

/// Names that contain a position word without being a position: dynamics,
/// errors, simulation flags.
fn disqualified_position_name(lower: &str) -> bool {
    const BAD: &[&str] = &[
        "error",
        "status",
        "valid",
        "quality",
        "satellite",
        "satellites",
        "speed",
        "accel",
        "acceleration",
        " jerk",
        "rate",
        "dop",
        "fix",
        "age",
        "sigma",
        "std",
        "noise",
        "sim",
    ];
    BAD.iter().any(|b| lower.contains(b))
}

/// Converts an error into a [`JsValue`] carrying the error message as a thrown JavaScript error.
fn js_err(err: impl std::fmt::Display) -> JsValue {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        JsError::new(&err.to_string()).into()
    }
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = err;
        JsValue::NULL
    }
}

/// Escapes a string for JSON output according to RFC 8259 and appends it to `out`.
///
/// Handles quotation marks, reverse solidi, standard escape characters (`\b`, `\f`, `\n`, `\r`, `\t`),
/// and control characters below 0x20 formatted as `\u00XX`.
pub fn escape_json_str_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Escapes a string for JSON output according to RFC 8259.
pub fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_json_str_into(s, &mut out);
    out
}

/// Appends an `f64` value to a JSON output buffer.
///
/// Finite numbers are formatted as decimal floating-point representations.
/// Non-finite numbers (`NaN`, `+inf`, `-inf`) are formatted as `null` as required by JSON.
pub fn write_f64(out: &mut String, val: f64) {
    if val.is_finite() {
        let _ = write!(out, "{}", val);
    } else {
        out.push_str("null");
    }
}

/// Appends an `f64` value to a CSV output buffer.
///
/// Unlike JSON, CSV has no `null`: a non-finite sample becomes an empty field,
/// which every spreadsheet and parser reads as a missing value.
fn write_csv_f64(out: &mut String, val: f64) {
    if val.is_finite() {
        let _ = write!(out, "{}", val);
    }
}

/// Appends a string as a single RFC 4180 CSV field, quoting it only when it
/// contains a comma, quote, or newline.
fn write_csv_field(out: &mut String, field: &str) {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        out.push('"');
        for c in field.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(field);
    }
}

/// Appends one sample's bytes as a quoted hex string (`"a1 2b …"`), ellipsized
/// past 16 bytes with the true length kept — a cell that silently hid most of
/// a 64-byte CAN FD payload would be its own small lie.
fn hex_field(sample: &[u8], out: &mut String) {
    const CAP: usize = 16;
    out.push('"');
    for (i, b) in sample.iter().take(CAP).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{b:02x}");
    }
    if sample.len() > CAP {
        let _ = write!(out, " … ({} B)", sample.len());
    }
    out.push('"');
}

/// Formats `(times[i], values[i])` pairs as a two-column CSV: a
/// `timestamp,<name>` header, then one row per sample. Non-finite values
/// become empty fields.
///
/// Pure so it can be tested natively without a JS runtime; [`WasmMf4File::signal_csv`]
/// only slices the decoded series and delegates here.
pub fn series_csv(times: &[f64], values: &[f64], name: &str) -> String {
    let mut out = String::with_capacity(times.len() * 24 + 16);
    out.push_str("timestamp,");
    write_csv_field(&mut out, name);
    out.push('\n');
    for (&t, &v) in times.iter().zip(values.iter()) {
        write_csv_f64(&mut out, t);
        out.push(',');
        write_csv_f64(&mut out, v);
        out.push('\n');
    }
    out
}

/// Decimates one run of finite samples into pixel columns, emitting at most
/// four points per column — first, min, max, last, in source order.
///
/// This is the `gui/src/decimate.rs` min/max algorithm, extended with the
/// column's first and last samples: min/max alone guarantees a spike survives,
/// and first/last additionally makes the polyline enter and exit the column at
/// real samples instead of jumping between neighbouring columns' extremes.
/// Emitting the four indices in ascending order (deduplicated) keeps the
/// output strictly time-ordered, so the canvas can stroke it as one path.
///
/// Like the reference implementation, column boundaries are recomputed from
/// the absolute timestamp of each column's first untouched sample (not a
/// running counter), so empty columns cannot desynchronize later columns from
/// their true boundaries, and the sample under the cursor is always consumed
/// before any boundary test so the loop always makes progress — a `col_end`
/// that rounds back to `x0` (a zoom narrower than the ulp of an
/// epoch-seconds master) then yields one-point columns instead of spinning.
// Append both arrays directly so each finite run needs no temporary output.
#[allow(clippy::too_many_arguments)]
fn decimate_run(
    times: &[f64],
    values: &[f64],
    x0: f64,
    span: f64,
    n_columns: usize,
    hard_cap: usize,
    out_t: &mut Vec<f64>,
    out_v: &mut Vec<f64>,
) {
    let col_width = span / n_columns as f64;
    // A span narrower than the representable width of a column has no
    // meaningful columns. That covers both a zero/negative width (identical
    // timestamps) and the ulp collapse of a zoom narrower than the spacing of
    // `x0` itself (an epoch-seconds master): there `x0 + col_width` rounds
    // straight back to `x0`, every column would degenerate to one sample, and
    // the budget would silently blow past itself. Keep the run's extremes
    // instead of emitting every duplicate sample.
    if col_width.partial_cmp(&0.0) != Some(Ordering::Greater)
        || !col_width.is_finite()
        || (x0 + col_width).partial_cmp(&x0) != Some(Ordering::Greater)
    {
        push_first_min_max_last(times, values, out_t, out_v);
        return;
    }

    let mut i = 0;
    while i < times.len() {
        if out_t.len() >= hard_cap {
            return;
        }
        // See the comment above: `col_index` is derived from the first
        // untouched sample, and that sample is consumed unconditionally.
        let col_index = ((times[i] - x0) / col_width) as usize;
        let col_end = x0 + (col_index as f64 + 1.0) * col_width;
        let first_i = i;
        let mut min_i = i;
        let mut max_i = i;
        i += 1;
        while i < times.len() && times[i] < col_end {
            if values[i] < values[min_i] {
                min_i = i;
            }
            if values[i] > values[max_i] {
                max_i = i;
            }
            i += 1;
        }
        let last_i = i - 1;

        // Ascending, deduplicated: at most four points, in time order.
        let mut idx = [first_i, min_i, max_i, last_i];
        idx.sort_unstable();
        for slot in 0..idx.len() {
            if slot > 0 && idx[slot] == idx[slot - 1] {
                continue;
            }
            out_t.push(times[idx[slot]]);
            out_v.push(values[idx[slot]]);
        }
    }
}

/// Pushes the first, min, max and last sample of a (finite) run, the
/// degenerate-column fallback of [`decimate_run`].
fn push_first_min_max_last(
    times: &[f64],
    values: &[f64],
    out_t: &mut Vec<f64>,
    out_v: &mut Vec<f64>,
) {
    if times.is_empty() || values.is_empty() {
        return;
    }
    let mut min_i = 0;
    let mut max_i = 0;
    for (i, &v) in values.iter().enumerate() {
        if v < values[min_i] {
            min_i = i;
        }
        if v > values[max_i] {
            max_i = i;
        }
    }
    let last_i = values.len() - 1;
    let mut idx = [0, min_i, max_i, last_i];
    idx.sort_unstable();
    for slot in 0..idx.len() {
        if slot > 0 && idx[slot] == idx[slot - 1] {
            continue;
        }
        out_t.push(times[idx[slot]]);
        out_v.push(values[idx[slot]]);
    }
}

/// A one-line, human-readable description of a channel's conversion rule —
/// the details panel's "what happens to a raw sample" line. Counts and
/// parameters are shown, coefficients are not (a six-term polynomial in a
/// tooltip is noise, not information).
pub fn describe_conversion(conversion: &falcon_mdf::Conversion) -> String {
    use falcon_mdf::Conversion as C;
    match conversion {
        C::None => "identity (raw = physical)".into(),
        C::Linear { offset, factor } => format!("linear: y = {factor}·x + {offset}"),
        C::Rational { .. } => "rational polynomial (6 coefficients)".into(),
        C::Algebraic { formula, .. } => format!("algebraic: {formula}"),
        C::TableInterpolated { keys, .. } => {
            format!("value→value table, {} points, interpolated", keys.len())
        }
        C::TableLookup { keys, .. } => format!("value→value table, {} points", keys.len()),
        C::RangeTable { lower, .. } => format!("value-range→value table, {} ranges", lower.len()),
        C::ValueToText { entries, .. } => format!("value→text table, {} entries", entries.len()),
        C::RangeToText { entries, .. } => {
            format!("value-range→text table, {} entries", entries.len())
        }
        C::TextToValue { keys, .. } => format!("text→value table, {} entries", keys.len()),
        C::TextToText { keys, .. } => format!("text→text table, {} entries", keys.len()),
        C::Bitfield { entries, .. } => format!("bitfield→text table, {} entries", entries.len()),
        other => format!("unsupported: {other:?}"),
    }
}

/// Restricts `(times[i], values[i])` to `[t0, t1]` and decimates the result to
/// at most `max_points` points, so the browser never receives more points
/// than it draws.
///
/// Behaviour, in order:
///
/// - A window covering no samples (including `t0 > t1`, or either bound
///   non-finite on an empty series) yields two empty vectors.
/// - Non-finite bounds are clamped to the series' own extent, so a viewer can
///   bootstrap its initial view with `(-Infinity, Infinity)`.
/// - When the window holds `max_points` samples or fewer, the visible samples
///   are returned untouched: there is nothing to aggregate away, and returning
///   fewer points than were asked for would just be a different way of lying
///   about what the file contains.
/// - Otherwise the window is divided into `max_points / 4` columns and each
///   column keeps its first, min, max and last sample
///   ([`decimate_run`]); a single-sample spike is always a column extreme, so
///   it always survives.
/// - Non-finite values (a NaN in the data, or a sample the file's
///   invalidation bits mark invalid — the reader folds both into the value)
///   neither contribute to a column's extremes nor get bridged by a line:
///   each run of them collapses to a single `NaN` point, which the drawing
///   side turns into a gap. Finite runs share the column budget in
///   proportion to their length, so many short runs cannot blow past
///   `max_points`; a 1.5× `hard_cap` guards the duplicate-timestamp corner
///   where columns degenerate to one sample each.
///
/// `times` must be sorted ascending, as every MDF master channel is. A
/// non-monotonic master (duplicate or backwards timestamps) cannot panic or
/// hang this function — the worst case is a conservative extra column.
pub fn decimate_window(
    times: &[f64],
    values: &[f64],
    t0: f64,
    t1: f64,
    max_points: usize,
) -> (Vec<f64>, Vec<f64>) {
    if times.is_empty()
        || values.len() != times.len()
        || max_points == 0
        || matches!(t0.partial_cmp(&t1), None | Some(Ordering::Greater))
    {
        return (Vec::new(), Vec::new());
    }
    // Clamp non-finite bounds to the data's extent (the initial full view).
    let x0 = if t0.is_finite() { t0 } else { times[0] };
    let x1 = if t1.is_finite() {
        t1
    } else {
        times[times.len() - 1]
    };
    if matches!(x0.partial_cmp(&x1), None | Some(Ordering::Greater)) {
        return (Vec::new(), Vec::new());
    }

    let start = times.partition_point(|&t| t < x0);
    let end = times.partition_point(|&t| t <= x1);
    if start >= end {
        return (Vec::new(), Vec::new());
    }
    if end - start <= max_points {
        return (times[start..end].to_vec(), values[start..end].to_vec());
    }

    let n_columns = (max_points / 4).max(1);
    let hard_cap = max_points + max_points / 2;

    // One pass over the window, splitting it into runs of finite samples and
    // runs of non-finite ones; the finite runs are decimated, the non-finite
    // runs each collapse to one NaN point that breaks the drawn line.
    let mut out_t = Vec::with_capacity(max_points.min(end - start));
    let mut out_v = Vec::with_capacity(out_t.capacity());
    let mut i = start;
    let mut finite_total: usize = 0;
    while i < end {
        if values[i].is_finite() {
            let run = i;
            while i < end && values[i].is_finite() {
                i += 1;
            }
            finite_total += i - run;
        } else {
            while i < end && !values[i].is_finite() {
                i += 1;
            }
        }
    }

    i = start;
    while i < end {
        if !values[i].is_finite() {
            let gap_t = times[i];
            while i < end && !values[i].is_finite() {
                i += 1;
            }
            out_t.push(gap_t);
            out_v.push(f64::NAN);
            continue;
        }
        let run = i;
        while i < end && values[i].is_finite() {
            i += 1;
        }
        let len = i - run;
        // u64 math: `n_columns * len` overflows u32 (wasm's usize) on large
        // files long before either factor does.
        let cols = ((n_columns as u64 * len as u64) / finite_total as u64).max(1) as usize;
        decimate_run(
            &times[run..i],
            &values[run..i],
            x0,
            x1 - x0,
            cols,
            hard_cap.saturating_sub(out_t.len()),
            &mut out_t,
            &mut out_v,
        );
    }
    (out_t, out_v)
}

/// Statistics over `(times[i], values[i])` inside `[t0, t1]`, as the JSON
/// [`WasmMf4File::signal_stats`] emits: `{count, invalid, min, max, mean,
/// first, last, t0, t1}`.
///
/// The window semantics are [`decimate_window`]'s, so every windowed endpoint
/// agrees about which samples a given `[t0, t1]` covers: bounds are
/// inclusive, non-finite bounds clamp to the series' extent, and a reversed
/// window, a NaN bound, or an empty series covers nothing. A non-finite
/// sample (the reader folds invalidation bits into NaN) is kept out of every
/// figure but counted in `invalid` — exactly the samples decimation collapses
/// into gap points. `first`/`last` are the first and last *finite* samples in
/// the window, so a Δ(value) between two cursors stays computable when the
/// sample under a cursor happens to be invalid. A window without finite
/// samples is a valid `count: 0` payload with null figures, not an error:
/// cursors may legitimately land inside a gap.
///
/// Pure so it can be tested natively without a JS runtime.
pub fn window_stats_json(times: &[f64], values: &[f64], t0: f64, t1: f64) -> String {
    let mut count = 0usize;
    let mut invalid = 0usize;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0;
    let mut first: Option<f64> = None;
    let mut last: Option<f64> = None;
    // The window as applied, echoed in the payload: same guards and the same
    // extent clamp as decimate_window, which is why a viewer may bootstrap
    // with (-Infinity, Infinity) here too.
    let mut x0 = t0;
    let mut x1 = t1;
    if !times.is_empty() && values.len() == times.len() && t0 <= t1 {
        x0 = if t0.is_finite() { t0 } else { times[0] };
        x1 = if t1.is_finite() {
            t1
        } else {
            times[times.len() - 1]
        };
        if x0 <= x1 {
            let start = times.partition_point(|&t| t < x0);
            let end = times.partition_point(|&t| t <= x1);
            for &v in values.iter().take(end).skip(start) {
                if !v.is_finite() {
                    invalid += 1;
                    continue;
                }
                count += 1;
                if v < min {
                    min = v;
                }
                if v > max {
                    max = v;
                }
                sum += v;
                if first.is_none() {
                    first = Some(v);
                }
                last = Some(v);
            }
        }
    }
    // A zero count leaves min/max at their fold seeds and mean at 0/0 = NaN,
    // so write_f64's non-finite → null rule emits the required null figures
    // without a branch per field.
    let mut out = String::with_capacity(96);
    out.push_str("{\"count\":");
    let _ = write!(out, "{}", count);
    out.push_str(",\"invalid\":");
    let _ = write!(out, "{}", invalid);
    out.push_str(",\"min\":");
    write_f64(&mut out, min);
    out.push_str(",\"max\":");
    write_f64(&mut out, max);
    out.push_str(",\"mean\":");
    write_f64(&mut out, sum / count as f64);
    out.push_str(",\"first\":");
    write_f64(&mut out, first.unwrap_or(f64::NAN));
    out.push_str(",\"last\":");
    write_f64(&mut out, last.unwrap_or(f64::NAN));
    out.push_str(",\"t0\":");
    write_f64(&mut out, x0);
    out.push_str(",\"t1\":");
    write_f64(&mut out, x1);
    out.push('}');
    out
}

/// How a channel's samples decode, as the string the viewer protocol uses:
/// `"f64"` for one number per sample, `"text"` for one label per sample,
/// `"bytes"` for opaque per-sample bytes, `"array"` for several values per
/// sample.
///
/// Metadata-only, so a channel list can mark its entries without decoding
/// anything. The exotic non-numeric scalars (complex, the CANopen kinds) pass
/// their own kind name through rather than being lumped into one of the four:
/// a badge that said "bytes" about a CANopen date would be its own small lie.
/// A viewer treats every kind other than `"f64"` and `"text"` as unplotable as
/// a scalar.
pub fn kind_of_channel(channel: &Channel) -> &'static str {
    if channel.array_shape.is_some() {
        return "array";
    }
    // A VLSD or maximum-length channel's record holds an offset (VLSD) or a
    // byte count (MLSD), not the value — the payload's type is what the data
    // type describes. The decoder resolves string payloads to labels and
    // everything else to bytes (maximum length always to bytes, even for
    // string payloads), and the list must agree with what a decode produces.
    if matches!(
        channel.channel_type,
        ChannelType::VariableLength | ChannelType::MaxLength
    ) {
        return if channel.channel_type == ChannelType::VariableLength
            && channel.data_type.is_string()
        {
            "text"
        } else {
            "bytes"
        };
    }
    match channel.value_kind() {
        ValueKind::Str => "text",
        ValueKind::Bytes => "bytes",
        k if k.is_numeric() => "f64",
        k => k.name(),
    }
}

/// A text channel's samples restricted to `[t0, t1]` and collapsed to its
/// state changes, as `(timestamps, labels, truncated)`.
///
/// A label series' irreducible content is its runs: consecutive samples
/// sharing a label are one band, however many samples it spans. So the
/// emission rule is the window's first sample, every label change, and the
/// window's last sample (so the final run closes at a real sample time) —
/// which is to label series what first/min/max/last per column is to numeric
/// ones: nothing a band view could still distinguish is dropped. The only
/// shape that defeats it is a label changing on (nearly) every sample — a
/// recording with more state changes than the viewer has pixels — and there
/// the `hard_cap` stops the output with `truncated` set, the same guard
/// [`decimate_window`] puts on its own degenerate columns.
///
/// `None` labels (an invalid sample — the text analog of the scalar path's
/// NaN fold) are emitted like any label: the drawing side leaves them
/// unpainted, the way it gaps a NaN.
///
/// Window semantics are [`decimate_window`]'s: inclusive bounds, non-finite
/// bounds clamped to the series' extent, a reversed or empty window yielding
/// nothing. Pure so it can be tested natively without a JS runtime.
pub fn decimate_label_runs(
    times: &[f64],
    labels: &[Option<&str>],
    t0: f64,
    t1: f64,
    max_points: usize,
) -> (Vec<f64>, Vec<Option<String>>, bool) {
    if times.is_empty()
        || labels.len() != times.len()
        || max_points == 0
        || matches!(t0.partial_cmp(&t1), None | Some(Ordering::Greater))
    {
        return (Vec::new(), Vec::new(), false);
    }
    let x0 = if t0.is_finite() { t0 } else { times[0] };
    let x1 = if t1.is_finite() {
        t1
    } else {
        times[times.len() - 1]
    };
    if matches!(x0.partial_cmp(&x1), None | Some(Ordering::Greater)) {
        return (Vec::new(), Vec::new(), false);
    }
    let start = times.partition_point(|&t| t < x0);
    let end = times.partition_point(|&t| t <= x1);
    if start >= end {
        return (Vec::new(), Vec::new(), false);
    }

    let hard_cap = max_points + max_points / 2;
    let mut out_t = Vec::with_capacity(max_points.min(end - start));
    let mut out_l: Vec<Option<String>> = Vec::with_capacity(out_t.capacity());
    let mut truncated = false;
    let mut last_label: Option<&str> = None;
    for i in start..end {
        if i != start && labels[i] == last_label {
            continue;
        }
        if out_t.len() >= hard_cap {
            truncated = true;
            break;
        }
        out_t.push(times[i]);
        out_l.push(labels[i].map(str::to_string));
        last_label = labels[i];
    }
    // The window's last sample closes the final run. Without it a run
    // spanning the window's end would stop one sample short and the band
    // would not reach the right edge.
    if !truncated && out_t.last() != Some(&times[end - 1]) {
        if out_t.len() >= hard_cap {
            truncated = true;
        } else {
            out_t.push(times[end - 1]);
            out_l.push(labels[end - 1].map(str::to_string));
        }
    }
    (out_t, out_l, truncated)
}

/// Label distribution over `[t0, t1]`, as the JSON [`WasmMf4File::signal_stats`]
/// emits for a text channel: `{count, invalid, t0, t1, labels: [{label,
/// samples, seconds}, …]}`, entries sorted by `seconds` descending.
///
/// The window semantics are [`decimate_window`]'s (inclusive bounds, extent
/// clamp for non-finite ones, reversed/NaN bounds covering nothing), so a
/// cursor region agrees about which samples it spans. `seconds` is the time a
/// label was active: each sample holds until the next one, so sample `i`
/// contributes `t[i+1] - t[i]` to its label — and the window's last sample
/// contributes nothing, because its end (the next sample, or the recording's
/// end) is outside the window and inventing it would add time the file does
/// not account for. `None` labels count in `invalid` and nowhere else.
///
/// Pure so it can be tested natively without a JS runtime.
pub fn window_label_stats_json(times: &[f64], labels: &[Option<&str>], t0: f64, t1: f64) -> String {
    let mut count = 0usize;
    let mut invalid = 0usize;
    // Distinct labels are few (a state channel's vocabulary, not its sample
    // count), so a linear-scan vec beats a map and keeps a stable sort key.
    let mut dist: Vec<(String, usize, f64)> = Vec::new();
    let mut x0 = t0;
    let mut x1 = t1;
    if !times.is_empty() && labels.len() == times.len() && t0 <= t1 {
        x0 = if t0.is_finite() { t0 } else { times[0] };
        x1 = if t1.is_finite() {
            t1
        } else {
            times[times.len() - 1]
        };
        if x0 <= x1 {
            let start = times.partition_point(|&t| t < x0);
            let end = times.partition_point(|&t| t <= x1);
            for i in start..end {
                let Some(label) = labels[i] else {
                    invalid += 1;
                    continue;
                };
                count += 1;
                let idx = match dist.iter().position(|(l, _, _)| l == label) {
                    Some(idx) => idx,
                    None => {
                        dist.push((label.to_string(), 0, 0.0));
                        dist.len() - 1
                    }
                };
                if let Some((_, samples, seconds)) = dist.get_mut(idx) {
                    *samples += 1;
                    if i + 1 < end {
                        let dt = times[i + 1] - times[i];
                        if dt > 0.0 {
                            *seconds += dt;
                        }
                    }
                }
            }
        }
    }
    // Most-active first; ties by name so two calls over one window agree.
    dist.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut out = String::with_capacity(96 + dist.len() * 48);
    out.push_str("{\"count\":");
    let _ = write!(out, "{}", count);
    out.push_str(",\"invalid\":");
    let _ = write!(out, "{}", invalid);
    out.push_str(",\"t0\":");
    write_f64(&mut out, x0);
    out.push_str(",\"t1\":");
    write_f64(&mut out, x1);
    out.push_str(",\"labels\":[");
    for (i, (label, samples, seconds)) in dist.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"label\":\"");
        escape_json_str_into(label, &mut out);
        out.push_str("\",\"samples\":");
        let _ = write!(out, "{}", samples);
        out.push_str(",\"seconds\":");
        write_f64(&mut out, *seconds);
        out.push('}');
    }
    out.push_str("]}");
    out
}

/// Extracts element `element` of every sample from a flat array decode, as one
/// `f64` per sample.
///
/// A fixed-shape array strides `elements_per_sample`; a dynamic-shape array
/// carries per-sample starts (`starts[i]..starts[i+1]` is sample `i`). A
/// sample without that element — only possible in the dynamic case — yields
/// `NaN`: an absent element is a gap in the drawn line, not a zero that would
/// read as a measurement.
///
/// Pure so it can be tested natively; [`WasmMf4File::signal_element_window`]
/// decimates the result exactly as it would a scalar channel.
pub fn element_values(
    values: &[f64],
    starts: Option<&[usize]>,
    elements_per_sample: usize,
    element: usize,
) -> Vec<f64> {
    match starts {
        None => {
            // A declared shape of zero elements has no sample count to divide
            // out; the reader refuses those channels before this runs, and an
            // inconsistent flat buffer must not panic here.
            if elements_per_sample == 0 {
                return Vec::new();
            }
            let n = values.len() / elements_per_sample;
            (0..n)
                .map(|i| {
                    values
                        .get(i * elements_per_sample + element)
                        .copied()
                        .unwrap_or(f64::NAN)
                })
                .collect()
        }
        Some(starts) => (0..starts.len().saturating_sub(1))
            .map(|i| {
                let from = starts[i];
                let to = starts.get(i + 1).copied().unwrap_or(from);
                if element < to.saturating_sub(from) {
                    values.get(from + element).copied().unwrap_or(f64::NAN)
                } else {
                    f64::NAN
                }
            })
            .collect(),
    }
}

/// Formats `(times[i], labels[i])` pairs as a two-column CSV: a
/// `timestamp,<name>` header, then one row per sample. An invalid sample
/// (`None` label) becomes an empty field, matching `series_csv`'s rule for
/// non-finite numbers.
///
/// Pure so it can be tested natively; [`WasmMf4File::signal_csv`] slices the
/// decoded labels and delegates here.
pub fn labels_csv(times: &[f64], labels: &[Option<&str>], name: &str) -> String {
    let mut out = String::with_capacity(times.len() * 24 + 16);
    out.push_str("timestamp,");
    write_csv_field(&mut out, name);
    out.push('\n');
    for (&t, l) in times.iter().zip(labels.iter()) {
        write_csv_f64(&mut out, t);
        out.push(',');
        if let Some(label) = l {
            write_csv_field(&mut out, label);
        }
        out.push('\n');
    }
    out
}

/// A channel's decoded samples, kept as `f64` for the typed-array and
/// decimation paths.
struct CachedSeries {
    unit: String,
    timestamps: Vec<f64>,
    values: Vec<f64>,
    /// What the samples are beyond the `f64` view — the part `to_f64` cannot
    /// say honestly (labels, array shape, opaque bytes).
    payload: Payload,
}

impl Payload {
    /// The kind string [`kind_of_channel`] would report for this decode.
    fn name(&self) -> &'static str {
        match self {
            Payload::Scalar => "f64",
            Payload::Text(_) => "text",
            Payload::Array { .. } | Payload::ArrayVarLen { .. } => "array",
            Payload::Bytes { .. } | Payload::VarBytes { .. } => "bytes",
        }
    }
}

/// What one channel decoded to, for choosing the payload path.
///
/// `CachedSeries::values` keeps `to_f64()`'s output for every kind — the
/// scalar endpoints (`signal_arrays`/`signal_window`) are defined over it and
/// their behavior must not change — while this carries what that view cannot
/// express.
#[derive(Clone)]
enum Payload {
    /// One `f64` per sample: `values` is the series.
    Scalar,
    /// One label per sample; `None` marks an invalid sample, the text analog
    /// of the scalar path's NaN fold.
    Text(Vec<Option<String>>),
    /// A fixed-shape array: `values` holds `elements_per_sample` values per
    /// sample, flat (element `j` of sample `i` at `i * elements_per_sample + j`).
    /// The flat cache is the seam a per-sample table view (plan 2.2) slices
    /// from later; `signal_element_window` is its scalar window today.
    Array { elements_per_sample: usize },
    /// A dynamic-shape array: `values` flat, one start per sample plus a
    /// final end marker.
    ArrayVarLen { starts: Vec<usize> },
    /// Fixed-width opaque bytes, flat (`width` bytes per sample). The bytes
    /// ride along so the sample table can show them honestly instead of the
    /// all-NaN view `to_f64` produces.
    Bytes { data: Vec<u8>, width: usize },
    /// Variable-width opaque bytes, one start per sample plus a final end.
    VarBytes { data: Vec<u8>, starts: Vec<usize> },
}

/// How many decoded channels to keep. The viewer owns its `WasmMf4File` from
/// one dedicated worker, so a plain FIFO is enough to make zoom/pan
/// re-requests reuse the previous decode; eight is the viewer's channel
/// overlay limit plus slack.
const SERIES_CACHE_CAP: usize = 10;

/// Folds the file's per-sample invalidation bits into the values: an invalid
/// sample becomes `NaN`, the same marker the data itself uses, so one code
/// path (drawing gap, decimation run split, empty CSV field) handles both.
///
/// A validity vector whose length doesn't match the samples can't be lined up
/// with them, so it is ignored rather than trusted (same stance as the GUI's
/// decimator).
fn fold_validity(values: &mut [f64], validity: Option<&[bool]>) {
    if let Some(valid) = validity {
        if valid.len() == values.len() {
            for (v, ok) in values.iter_mut().zip(valid.iter()) {
                if !ok {
                    *v = f64::NAN;
                }
            }
        }
    }
}

/// Whether sample `i` is valid, with [`fold_validity`]'s stance on a validity
/// vector that cannot be lined up with the samples: ignored, all valid.
fn valid_at(validity: Option<&[bool]>, i: usize, len: usize) -> bool {
    match validity {
        Some(v) if v.len() == len => v.get(i).copied().unwrap_or(false),
        _ => true,
    }
}

/// A computed channel's parsed expression (plan 4.2).
///
/// The grammar is deliberately small — numbers, the four arithmetic
/// operators, unary minus, parentheses, `abs`, `sqrt`, `min`, `max`, and
/// channel references written `[ChannelName]` (the GUI's spelling). Anything
/// else is a parse error naming the position, never a silent zero.
#[derive(Clone)]
struct Expr {
    source: String,
    root: Node,
    refs: Vec<String>,
}

#[derive(Clone)]
enum Node {
    Num(f64),
    Ref(usize), // index into Expr::refs
    Neg(Box<Node>),
    Add(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
    Mul(Box<Node>, Box<Node>),
    Div(Box<Node>, Box<Node>),
    Abs(Box<Node>),
    Sqrt(Box<Node>),
    Min(Box<Node>, Box<Node>),
    Max(Box<Node>, Box<Node>),
}

struct Parser<'a> {
    chars: Vec<(usize, char)>, // (byte-ish position, char) for error messages
    pos: usize,
    src: &'a str,
    refs: Vec<String>,
}

impl<'a> Parser<'a> {
    fn parse(source: &'a str) -> Result<Expr, String> {
        let chars: Vec<(usize, char)> = source.char_indices().collect();
        let mut p = Parser {
            chars,
            pos: 0,
            src: source,
            refs: Vec::new(),
        };
        let root = p.expr()?;
        p.skip_ws();
        if p.pos < p.chars.len() {
            return Err(format!(
                "unexpected '{}' at offset {} — the expression ends here",
                p.chars[p.pos].1, p.chars[p.pos].0
            ));
        }
        Ok(Expr {
            source: source.to_string(),
            root,
            refs: p.refs,
        })
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).map(|(_, c)| *c)
    }

    fn skip_ws(&mut self) {
        while self.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
            self.pos += 1;
        }
    }

    fn expr(&mut self) -> Result<Node, String> {
        let mut left = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    left = Node::Add(Box::new(left), Box::new(self.term()?));
                }
                Some('-') => {
                    self.pos += 1;
                    left = Node::Sub(Box::new(left), Box::new(self.term()?));
                }
                _ => return Ok(left),
            }
        }
    }

    fn term(&mut self) -> Result<Node, String> {
        let mut left = self.unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    left = Node::Mul(Box::new(left), Box::new(self.unary()?));
                }
                Some('/') => {
                    self.pos += 1;
                    left = Node::Div(Box::new(left), Box::new(self.unary()?));
                }
                _ => return Ok(left),
            }
        }
    }

    fn unary(&mut self) -> Result<Node, String> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.pos += 1;
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Node, String> {
        self.skip_ws();
        let Some(&(at, c)) = self.chars.get(self.pos) else {
            return Err("unexpected end of expression".into());
        };
        if c == '(' {
            self.pos += 1;
            let inner = self.expr()?;
            self.skip_ws();
            if self.peek() == Some(')') {
                self.pos += 1;
                return Ok(inner);
            }
            return Err(format!("missing ')' at offset {at}"));
        }
        if c == '[' {
            // A channel reference: everything up to the matching ']'.
            self.pos += 1;
            let start = self.pos;
            while self.peek().map(|ch| ch != ']').unwrap_or(false) {
                self.pos += 1;
            }
            let end = self.pos;
            if self.peek() != Some(']') {
                return Err(format!(
                    "unterminated channel name, missing ']' after offset {start}"
                ));
            }
            self.pos += 1;
            let name: String = self.chars[start..end].iter().map(|(_, c)| *c).collect();
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(format!("empty channel reference at offset {at}"));
            }
            if let Some(i) = self.refs.iter().position(|r| *r == name) {
                return Ok(Node::Ref(i));
            }
            self.refs.push(name);
            return Ok(Node::Ref(self.refs.len() - 1));
        }
        if c.is_ascii_alphabetic() {
            // A function name.
            let start = self.pos;
            while self
                .peek()
                .map(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                .unwrap_or(false)
            {
                self.pos += 1;
            }
            let name: String = self.chars[start..self.pos]
                .iter()
                .map(|(_, c)| *c)
                .collect();
            self.skip_ws();
            if self.peek() != Some('(') {
                return Err(format!("unknown token '{name}' at offset {start} — channel references are written [name]"));
            }
            self.pos += 1;
            let a = self.expr()?;
            self.skip_ws();
            let args = match self.peek() {
                Some(',') => {
                    self.pos += 1;
                    let b = self.expr()?;
                    vec![a, b]
                }
                _ => vec![a],
            };
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(format!(
                    "missing ')' for {name} at offset {}",
                    self.chars[self.pos.min(self.chars.len() - 1)].0
                ));
            }
            self.pos += 1;
            return match (name.as_str(), args.len()) {
                // Destructured, never unwrap(): this crate may not panic.
                ("abs", 1) => {
                    let mut it = args.into_iter();
                    match (it.next(), it.next()) {
                        (Some(a), None) => Ok(Node::Abs(Box::new(a))),
                        _ => Err("abs argument mismatch".into()),
                    }
                }
                ("sqrt", 1) => {
                    let mut it = args.into_iter();
                    match (it.next(), it.next()) {
                        (Some(a), None) => Ok(Node::Sqrt(Box::new(a))),
                        _ => Err("sqrt argument mismatch".into()),
                    }
                }
                ("min", 2) | ("max", 2) => {
                    let mut it = args.into_iter();
                    let a = it.next().unwrap();
                    let b = it.next().unwrap();
                    Ok(if name == "min" {
                        Node::Min(Box::new(a), Box::new(b))
                    } else {
                        Node::Max(Box::new(a), Box::new(b))
                    })
                }
                (n, k) => Err(format!(
                    "{n} with {k} argument(s) at offset {start} — abs/sqrt take one, min/max take two"
                )),
            };
        }
        // A number: digits with optional fraction and exponent.
        let start = self.pos;
        while self
            .peek()
            .map(|ch| ch.is_ascii_digit() || ch == '.')
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        if self.peek() == Some('e') || self.peek() == Some('E') {
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            while self.peek().map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos]
            .iter()
            .map(|(_, c)| *c)
            .collect();
        if let Ok(v) = text.parse::<f64>() {
            let _ = at;
            return Ok(Node::Num(v));
        }
        Err(format!(
            "unexpected '{}' at offset {start}",
            self.src.chars().nth(start).unwrap_or('?')
        ))
    }
}

impl Expr {
    fn eval(&self, node: &Node, lookup: &dyn Fn(usize) -> f64) -> f64 {
        match node {
            Node::Num(v) => *v,
            Node::Ref(i) => lookup(*i),
            Node::Neg(a) => -self.eval(a, lookup),
            Node::Add(a, b) => self.eval(a, lookup) + self.eval(b, lookup),
            Node::Sub(a, b) => self.eval(a, lookup) - self.eval(b, lookup),
            Node::Mul(a, b) => self.eval(a, lookup) * self.eval(b, lookup),
            // Float division by zero is inf/NaN, not a panic — a computed
            // channel may legitimately blow up where its inputs do.
            Node::Div(a, b) => self.eval(a, lookup) / self.eval(b, lookup),
            Node::Abs(a) => self.eval(a, lookup).abs(),
            Node::Sqrt(a) => self.eval(a, lookup).sqrt(),
            Node::Min(a, b) => self.eval(a, lookup).min(self.eval(b, lookup)),
            Node::Max(a, b) => self.eval(a, lookup).max(self.eval(b, lookup)),
        }
    }
}

/// An MF4 file held in browser memory.
#[wasm_bindgen]
pub struct WasmMf4File {
    inner: Inner,
    series_cache: Vec<(String, CachedSeries)>,
    /// Decoded bus channels (plan 3.2): the overlay a DBC attach produced.
    /// Separate from the LRU `series_cache` because a dropped bus channel
    /// cannot be re-decoded lazily — only by re-running the attach.
    bus: Vec<BusChannel>,
    /// Computed channels (plan 4.2): user expressions over existing channels,
    /// evaluated in Rust on raw arrays. Outside the LRU like bus channels,
    /// but re-derivable, so an eviction is only ever a recomputation.
    computed: Vec<(String, Expr)>,
}

/// One DBC-decoded channel: its viewer-facing name, where it came from, and
/// the decoded series itself.
struct BusChannel {
    name: String,
    message: String,
    series: CachedSeries,
}

impl BusChannel {
    fn kind(&self) -> &'static str {
        self.series.payload.name()
    }
}

#[wasm_bindgen]
impl WasmMf4File {
    /// Reads a file from bytes, e.g. a `Uint8Array` from `fetch` or a file input.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>) -> Result<WasmMf4File, JsValue> {
        // Signature sniff: both formats open with "MDF     " (or "UnFinMF "
        // for an unfinalized v4), so the discriminator is the format's major
        // version digit at byte 8 — '4' for MDF 4, '2'/'3' for the MDF 3
        // reader (which owns 2.x as well). Anything else goes to the v4
        // parser so the error message stays the one it always threw.
        let major = bytes.get(8).copied().unwrap_or(b'4');
        let inner = if major == b'2' || major == b'3' {
            Inner::V3(Mdf3File::from_source(Arc::new(MemorySource::new(bytes))).map_err(js_err)?)
        } else {
            Inner::V4(Mf4File::from_bytes(bytes).map_err(js_err)?)
        };
        Ok(WasmMf4File {
            inner,
            series_cache: Vec::new(),
            bus: Vec::new(),
            computed: Vec::new(),
        })
    }

    /// Resolves any channel name — file, DBC-decoded or computed — to its
    /// series without touching the LRU. `decoded()` is this plus the cache;
    /// endpoints that only read once (`signal`, computed recursion) skip the
    /// cache bookkeeping entirely.
    fn fetch_series(&self, name: &str) -> Result<CachedSeries, JsValue> {
        if let Some(bus) = self.bus.iter().find(|b| b.name == name) {
            return Ok(CachedSeries {
                unit: bus.series.unit.clone(),
                timestamps: bus.series.timestamps.clone(),
                values: bus.series.values.clone(),
                payload: bus.series.payload.clone(),
            });
        }
        if self.computed.iter().any(|(n, _)| n == name) {
            return self.decode_computed(name);
        }
        match &self.inner {
            Inner::V4(file) => decode_v4(file, name),
            Inner::V3(file) => decode_v3(file, name),
        }
    }

    /// Decodes `name` once and caches it, folding the file's per-sample
    /// invalidation bits into the values (an invalid sample becomes `NaN`, the
    /// same marker the data itself uses, so one code path handles both).
    fn decoded(&mut self, name: &str) -> Result<&CachedSeries, JsValue> {
        if let Some(pos) = self.series_cache.iter().position(|(n, _)| n == name) {
            // Move-to-front: the channels a viewer keeps zooming are the ones
            // it just asked for.
            let entry = self.series_cache.remove(pos);
            self.series_cache.push(entry);
            // Just re-pushed, so last() is Some; ok_or keeps the whole crate
            // panic-free even if that invariant ever breaks.
            return self
                .series_cache
                .last()
                .map(|entry| &entry.1)
                .ok_or_else(|| js_err("series cache corrupted"));
        }

        if self.computed.iter().any(|(n, _)| n == name) {
            let entry = self.decode_computed(name)?;
            self.series_cache.push((name.to_string(), entry));
            if self.series_cache.len() >= SERIES_CACHE_CAP {
                self.series_cache.remove(0);
            }
            return self
                .series_cache
                .last()
                .map(|e| &e.1)
                .ok_or_else(|| js_err("series cache corrupted"));
        }
        if let Some(bus) = self.bus.iter().find(|b| b.name == name) {
            // Bus channels are permanent: a clone goes into the LRU so zooms
            // reuse it, while the overlay itself never evicts.
            let entry = CachedSeries {
                unit: bus.series.unit.clone(),
                timestamps: bus.series.timestamps.clone(),
                values: bus.series.values.clone(),
                payload: bus.series.payload.clone(),
            };
            self.series_cache.push((name.to_string(), entry));
            return self
                .series_cache
                .last()
                .map(|e| &e.1)
                .ok_or_else(|| js_err("series cache corrupted"));
        }
        let entry = self.fetch_series(name)?;
        self.series_cache.push((name.to_string(), entry));
        if self.series_cache.len() >= SERIES_CACHE_CAP {
            self.series_cache.remove(0);
        }
        self.series_cache
            .last()
            .map(|entry| &entry.1)
            .ok_or_else(|| js_err("series cache corrupted"))
    }

    /// Every channel name in the file, as a JSON array of strings.
    pub fn channel_names(&self) -> Result<String, JsValue> {
        let mut out = String::from("[");
        let mut first = true;
        let mut push = |name: &str, first: &mut bool| {
            if !*first {
                out.push(',');
            }
            *first = false;
            out.push('"');
            escape_json_str_into(name, &mut out);
            out.push('"');
        };
        for name in self.inner.channel_names() {
            push(&name, &mut first);
        }
        for bus in &self.bus {
            push(&bus.name, &mut first);
        }
        for (name, _) in &self.computed {
            push(name, &mut first);
        }
        out.push(']');
        Ok(out)
    }

    /// The number of channels — file channels plus any DBC-decoded ones.
    pub fn channel_count(&self) -> usize {
        self.inner.channel_count() + self.bus.len() + self.computed.len()
    }

    /// One channel's samples as a JSON object with `name`, `unit`,
    /// `timestamps` and `values` arrays.
    ///
    /// Non-finite floats (`NaN`, `+inf`, `-inf`) are not valid JSON and are emitted
    /// as `null` in both `timestamps` and `values` arrays.
    pub fn signal(&self, name: &str) -> Result<String, JsValue> {
        // Same shape for both formats: the scalar (to_f64) view of one
        // channel over its master's timestamps. A DBC-decoded channel comes
        // straight from its overlay series.
        // DBC-decoded and computed channels serve straight from their
        // overlays — no decode cache involved.
        if let Some(bus) = self.bus.iter().find(|b| b.name == name) {
            return signal_json(
                name,
                &bus.series.unit,
                &bus.series.timestamps,
                &bus.series.values,
            );
        }
        if self.computed.iter().any(|(n, _)| n == name) {
            let series = self.fetch_series(name)?;
            return signal_json(name, &series.unit, &series.timestamps, &series.values);
        }
        let (channel_name, unit, timestamps, values) = match &self.inner {
            Inner::V4(f) => {
                let channel = f
                    .find_channel(name)
                    .ok_or_else(|| Mf4Error::ChannelNotFound {
                        name: name.to_string(),
                    })
                    .map_err(js_err)?;
                let series = f.time_series(channel).map_err(js_err)?;
                (
                    channel.name.clone(),
                    channel.unit.clone(),
                    series.timestamps.to_vec(),
                    series.values.to_f64(),
                )
            }
            Inner::V3(f) => {
                let series = decode_v3(f, name)?;
                (
                    name.to_string(),
                    series.unit,
                    series.timestamps,
                    series.values,
                )
            }
        };

        signal_json(&channel_name, &unit, &timestamps, &values)
    }

    /// Version, start time, group and channel counts, as a JSON object.
    pub fn info(&self) -> Result<String, JsValue> {
        let channel_group_count: usize;
        let version;
        // An empty start time is the contract for "no wall clock": the
        // viewer falls back to a relative-only axis instead of guessing.
        let start_time: String = match &self.inner {
            Inner::V4(f) => {
                channel_group_count = f
                    .data_groups()
                    .iter()
                    .map(|dg| dg.channel_groups.len())
                    .sum();
                version = f.version().to_string();
                f.start_time().to_iso8601()
            }
            Inner::V3(f) => {
                channel_group_count = f
                    .data_groups()
                    .iter()
                    .map(|dg| dg.channel_groups.len())
                    .sum();
                version = f.version().to_string();
                // 3.20 and later carry an absolute time; earlier files only
                // have date/time text this binding does not parse.
                f.start_time_ns().map(ns_to_iso8601).unwrap_or_default()
            }
        };

        let mut out = String::new();
        out.push_str("{\"version\":\"");
        escape_json_str_into(&version, &mut out);
        out.push_str("\",\"start_time\":\"");
        escape_json_str_into(&start_time, &mut out);
        out.push_str("\",\"channel_group_count\":");
        let _ = write!(
            out,
            "{},\"channel_count\":{}",
            channel_group_count,
            self.inner.channel_count() + self.bus.len()
        );
        out.push('}');
        Ok(out)
    }

    /// Every channel's metadata in one JSON call, as an array of
    /// `{name, unit, group, description, kind}` objects — one metadata round
    /// trip instead of one `signal` call per channel just to learn the unit.
    ///
    /// The list matches [`WasmMf4File::channel_names`]: same order, one entry
    /// per channel (file channels first, then DBC-decoded ones).
    pub fn channels(&self) -> Result<String, JsValue> {
        let mut out = String::from("[");
        let mut first = true;
        let push = |out: &mut String, first: &mut bool| {
            if !*first {
                out.push(',');
            }
            *first = false;
        };
        for name in self.inner.channel_names() {
            push(&mut out, &mut first);
            let Some(kind) = self.inner.channel_kind(&name) else {
                continue;
            };
            out.push_str("{\"name\":\"");
            escape_json_str_into(&name, &mut out);
            out.push_str("\",\"kind\":\"");
            out.push_str(kind);
            match &self.inner {
                Inner::V4(f) => {
                    let Some(channel) = f.find_channel(&name) else {
                        continue;
                    };
                    out.push_str("\",\"unit\":\"");
                    escape_json_str_into(&channel.unit, &mut out);
                    out.push_str("\",\"group\":\"");
                    let group = f
                        .data_groups()
                        .get(channel.data_group_index)
                        .and_then(|dg| dg.channel_groups.get(channel.channel_group_index))
                        .map(|cg| cg.acquisition_name.trim())
                        .filter(|acq| !acq.is_empty());
                    match group {
                        Some(acq) => escape_json_str_into(acq, &mut out),
                        None => {
                            let _ = write!(
                                out,
                                "group {}.{}",
                                channel.data_group_index, channel.channel_group_index
                            );
                        }
                    }
                    out.push_str("\",\"description\":\"");
                    escape_json_str_into(&channel.comment, &mut out);
                }
                Inner::V3(f) => {
                    // The v3 group's comment is the closest thing it has to
                    // an acquisition name; the channel description is the
                    // CNBLOCK's identifier text.
                    let mut group = "";
                    let mut unit = "";
                    let mut description = "";
                    for dg in f.data_groups() {
                        for cg in &dg.channel_groups {
                            if let Some(ch) = cg.channels.iter().find(|ch| ch.name == name) {
                                group = cg.comment.trim();
                                unit = ch.unit.as_str();
                                description = ch.description.as_str();
                            }
                        }
                    }
                    out.push_str("\",\"unit\":\"");
                    escape_json_str_into(unit, &mut out);
                    out.push_str("\",\"group\":\"");
                    escape_json_str_into(group, &mut out);
                    out.push_str("\",\"description\":\"");
                    escape_json_str_into(description, &mut out);
                }
            }
            out.push_str("\"}");
        }
        // Computed channels ride the same list; the description IS the
        // expression, so the list says how every number was made.
        for (name, expr) in &self.computed {
            push(&mut out, &mut first);
            out.push_str("{\"name\":\"");
            escape_json_str_into(name, &mut out);
            out.push_str("\",\"unit\":\"\",\"group\":\"computed\",\"description\":\"");
            escape_json_str_into(&expr.source, &mut out);
            out.push_str("\",\"kind\":\"f64\"}");
        }
        // Decoded bus channels ride the same list: their group names the
        // DBC message they were decoded from.
        for bus in &self.bus {
            push(&mut out, &mut first);
            out.push_str("{\"name\":\"");
            escape_json_str_into(&bus.name, &mut out);
            out.push_str("\",\"unit\":\"");
            escape_json_str_into(&bus.series.unit, &mut out);
            out.push_str("\",\"group\":\"DBC · ");
            escape_json_str_into(&bus.message, &mut out);
            out.push_str("\",\"description\":\"DBC-decoded bus signal\",\"kind\":\"");
            out.push_str(bus.kind());
            out.push_str("\"}");
        }
        out.push(']');
        Ok(out)
    }

    /// Channel names matching `pattern`, as a JSON array of strings — the
    /// viewer's search box, so a filter over a file with thousands of
    /// channels runs against the reader's name index, not a shipped copy.
    ///
    /// `mode` selects the match: `"contains"` (case-insensitive substring —
    /// the default a plain query means), `"wildcard"` (`*` any run, `?` one
    /// character, whole-name), or `"exact"`. Regex deliberately lives on the
    /// JS side of the demo: `RegExp` is a platform primitive there, full
    /// (the reader's Rust regex subset in the GUI predates it), and costs
    /// this module zero bytes. DBC-decoded channels join the candidates.
    pub fn search_channels(&self, pattern: &str, mode: &str) -> Result<String, JsValue> {
        let mode_matches = |name: &str| match mode {
            "contains" => name.to_lowercase().contains(&pattern.to_lowercase()),
            "wildcard" => wildcard_match(name, pattern),
            "exact" => name == pattern,
            _ => false,
        };
        let mut names: Vec<String> = match mode {
            "contains" => match &self.inner {
                Inner::V4(f) => f.search_channels(pattern, SearchMode::CaseInsensitive),
                Inner::V3(f) => f
                    .channel_names()
                    .into_iter()
                    .filter(|n| n.to_lowercase().contains(&pattern.to_lowercase()))
                    .map(str::to_string)
                    .collect(),
            },
            "wildcard" => match &self.inner {
                Inner::V4(f) => f.search_channels(pattern, SearchMode::Wildcard),
                Inner::V3(f) => f
                    .channel_names()
                    .into_iter()
                    .filter(|n| wildcard_match(n, pattern))
                    .map(str::to_string)
                    .collect(),
            },
            "exact" => self
                .inner
                .channel_names()
                .into_iter()
                .filter(|n| *n == pattern)
                .collect(),
            other => return Err(js_err(format!("unknown search mode '{other}'"))),
        };
        let bus_names: Vec<String> = self
            .bus
            .iter()
            .map(|b| b.name.clone())
            .filter(|name| mode_matches(name))
            .collect();
        names.extend(bus_names);
        let mut out = String::from("[");
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_json_str_into(name, &mut out);
            out.push('"');
        }
        out.push(']');
        Ok(out)
    }

    /// How `name`'s samples decode — `"f64"`, `"text"`, `"bytes"` or `"array"`
    /// (the exotic non-numeric scalars name themselves: `"complex"`,
    /// `"canopen_date"`, `"canopen_time"`); see [`kind_of_channel`].
    ///
    /// Metadata-only, so asking costs no decode. [`WasmMf4File::channels`]
    /// carries the same string per entry when a caller wants them all at once.
    pub fn channel_kind(&self, name: &str) -> Result<String, JsValue> {
        if self.computed.iter().any(|(n, _)| n == name) {
            return Ok("f64".to_string());
        }
        if let Some(bus) = self.bus.iter().find(|b| b.name == name) {
            return Ok(bus.kind().to_string());
        }
        match self.inner.channel_kind(name) {
            Some(kind) => Ok(kind.to_string()),
            None => Err(js_err(Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })),
        }
    }

    /// One channel's metadata for the details panel, as a JSON object: name,
    /// unit, kind, description, group, the group's sample count, data type
    /// and bit count, the declared array shape (arrays only), declared
    /// min/max (when the file defines them), whether the channel is its
    /// group's master, and a one-line description of the conversion rule.
    pub fn channel_details(&self, name: &str) -> Result<String, JsValue> {
        if let Inner::V3(f) = &self.inner {
            let mut found = None;
            for dg in f.data_groups() {
                for cg in &dg.channel_groups {
                    if let Some(ch) = cg.channels.iter().find(|ch| ch.name == name) {
                        found = Some((ch, cg));
                    }
                }
            }
            let Some((ch, cg)) = found else {
                return Err(js_err(Mf4Error::ChannelNotFound {
                    name: name.to_string(),
                }));
            };
            let kind = if ch.data_type == 7 {
                "text"
            } else if ch.data_type == 8 {
                "bytes"
            } else {
                "f64"
            };
            let mut out = String::with_capacity(256);
            out.push_str("{\"name\":\"");
            escape_json_str_into(&ch.name, &mut out);
            out.push_str("\",\"unit\":\"");
            escape_json_str_into(&ch.unit, &mut out);
            out.push_str("\",\"kind\":\"");
            out.push_str(kind);
            out.push_str("\",\"description\":\"");
            escape_json_str_into(&ch.description, &mut out);
            out.push_str("\",\"group\":\"");
            escape_json_str_into(cg.comment.trim(), &mut out);
            out.push_str("\",\"samples\":");
            let _ = write!(out, "{}", cg.cycle_count);
            out.push_str(",\"data_type\":\"");
            let _ = write!(out, "MDF3 code {}", ch.data_type);
            out.push_str("\",\"bit_count\":");
            let _ = write!(out, "{}", ch.bit_count);
            out.push_str(",\"master\":");
            out.push_str(if ch.is_time() { "true" } else { "false" });
            out.push_str(",\"conversion\":\"");
            out.push_str(if ch.conversion_addr == 0 {
                "identity (raw = physical)"
            } else {
                "conversion block (applied by the reader)"
            });
            out.push_str("\"}");
            return Ok(out);
        }
        let Inner::V4(f) = &self.inner else {
            unreachable!("the v3 branch above returned");
        };
        let channel = f
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;
        let groups = f.data_groups();
        let group = groups
            .get(channel.data_group_index)
            .and_then(|dg| dg.channel_groups.get(channel.channel_group_index));

        let mut out = String::with_capacity(512);
        out.push_str("{\"name\":\"");
        escape_json_str_into(&channel.name, &mut out);
        out.push_str("\",\"unit\":\"");
        escape_json_str_into(&channel.unit, &mut out);
        out.push_str("\",\"kind\":\"");
        out.push_str(kind_of_channel(channel));
        out.push_str("\",\"description\":\"");
        escape_json_str_into(&channel.comment, &mut out);
        out.push_str("\",\"group\":\"");
        let acq = group
            .map(|cg| cg.acquisition_name.trim())
            .filter(|acq| !acq.is_empty());
        match acq {
            Some(acq) => escape_json_str_into(acq, &mut out),
            None => {
                let _ = write!(
                    out,
                    "group {}.{}",
                    channel.data_group_index, channel.channel_group_index
                );
            }
        }
        out.push_str("\",\"samples\":");
        // The group's cycle count — every channel in it shares it (see the
        // field's doc); a 0 means the file never said.
        let _ = write!(out, "{}", group.map(|cg| cg.sample_count).unwrap_or(0));
        out.push_str(",\"data_type\":\"");
        let _ = write!(out, "{:?}", channel.data_type);
        out.push_str("\",\"bit_count\":");
        let _ = write!(out, "{}", channel.bit_count);
        out.push_str(",\"master\":");
        out.push_str(if channel.is_master() { "true" } else { "false" });
        if let Some(dims) = channel.array_shape() {
            out.push_str(",\"array_shape\":[");
            for (i, &d) in dims.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{d}");
            }
            out.push(']');
        }
        if let Some(min) = channel.min_value {
            out.push_str(",\"min\":");
            write_f64(&mut out, min);
        }
        if let Some(max) = channel.max_value {
            out.push_str(",\"max\":");
            write_f64(&mut out, max);
        }
        out.push_str(",\"conversion\":\"");
        escape_json_str_into(&describe_conversion(&channel.conversion), &mut out);
        out.push_str("\"}");
        Ok(out)
    }

    /// One element of an array channel's shape, as a JSON array of its
    /// dimension sizes — `"[2,4]"` for a 2×4 matrix. `[]` for a scalar
    /// channel: asking a scalar for its shape is not an error, so a viewer
    /// can ask unconditionally.
    pub fn array_shape(&self, name: &str) -> Result<String, JsValue> {
        let Inner::V4(f) = &self.inner else {
            // MDF 3 has no array channels: the honest shape is the scalar one.
            let _ = name;
            return Ok("[]".to_string());
        };
        let channel = f
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;
        let mut out = String::from("[");
        if let Some(dims) = channel.array_shape() {
            for (i, &d) in dims.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", d);
            }
        }
        out.push(']');
        Ok(out)
    }

    /// A text channel's samples within `[t0, t1]` as JSON:
    /// `{"name", "unit", "kind":"text", "timestamps":[…], "labels":[…],
    /// "truncated":bool}`, one label per timestamp (`null` for an invalid
    /// sample) and run-collapsed to the label changes ([`decimate_label_runs`]
    /// — the label analog of `signal_window`'s point budget).
    ///
    /// Text here means the decoded labels, which includes channels made of
    /// text by a value↔text conversion: the reader resolves the table — keys,
    /// ranges, defaults, and the nested conversions some tables carry as
    /// entries — so the payload is the label the file defines, and the raw
    /// number a label came from is not shipped alongside. Re-resolving the
    /// table on the JS side would mean re-implementing that resolution
    /// (including the nested-conversion case); the label is the physical
    /// value, so the label is the whole payload.
    ///
    /// JSON rather than typed arrays because a label series carries strings —
    /// there is no `Float64Array` for those — and state channels are low-rate
    /// next to the numeric channels the typed-array endpoints exist for.
    pub fn signal_text(
        &mut self,
        name: &str,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<String, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            payload,
            ..
        } = self.decoded(name)?;
        let Payload::Text(labels) = payload else {
            return Err(js_err(format!(
                "channel '{}' decodes as {}, which has no labels; signal_text is for text channels",
                name,
                payload.name()
            )));
        };
        let refs: Vec<Option<&str>> = labels.iter().map(|l| l.as_deref()).collect();
        let (ts, ls, truncated) = decimate_label_runs(timestamps, &refs, t0, t1, max_points);

        let mut out = String::with_capacity(48 + ts.len() * 20);
        out.push_str("{\"name\":\"");
        escape_json_str_into(name, &mut out);
        out.push_str("\",\"unit\":\"");
        escape_json_str_into(unit, &mut out);
        out.push_str("\",\"kind\":\"text\",\"timestamps\":[");
        for (i, &t) in ts.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, t);
        }
        out.push_str("],\"labels\":[");
        for (i, l) in ls.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match l {
                Some(label) => {
                    out.push('"');
                    escape_json_str_into(label, &mut out);
                    out.push('"');
                }
                None => out.push_str("null"),
            }
        }
        out.push_str("],\"truncated\":");
        out.push_str(if truncated { "true" } else { "false" });
        out.push('}');
        Ok(out)
    }

    /// One channel's samples as `{timestamps: Float64Array, values: Float64Array,
    /// name, unit}`.
    ///
    /// Unlike [`WasmMf4File::signal`] (JSON, where non-finite floats must
    /// become `null`), a typed array round-trips `NaN` bit-for-bit — the
    /// drawing side turns those into gaps in the line.
    ///
    /// Array channels additionally carry their shape (`eps` for a fixed
    /// elements-per-sample count, `starts` for a dynamic one), so a caller
    /// holding the raw arrays can address single elements — the cursor
    /// readout of a plotted element needs exactly that.
    pub fn signal_arrays(&mut self, name: &str) -> Result<js_sys::Object, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            values,
            payload,
            ..
        } = self.decoded(name)?;
        let shape = match payload {
            Payload::Array {
                elements_per_sample,
            } => Some(Shape::Eps(*elements_per_sample)),
            Payload::ArrayVarLen { starts } => Some(Shape::Starts(starts.clone())),
            _ => None,
        };
        series_object(name, unit, timestamps, values, None, shape)
    }

    /// [`WasmMf4File::signal_arrays`] restricted to `[t0, t1]` and decimated in
    /// Rust to at most `max_points` points (first/min/max/last per column —
    /// see [`decimate_window`]), so a zoom or pan never ships more points to
    /// the main thread than it will draw.
    ///
    /// Non-finite bounds are clamped to the channel's extent, so a viewer can
    /// request its initial full view as `(-Infinity, Infinity)`.
    pub fn signal_window(
        &mut self,
        name: &str,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<js_sys::Object, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            values,
            ..
        } = self.decoded(name)?;
        let (ts, vs) = decimate_window(timestamps, values, t0, t1, max_points);
        series_object(name, unit, &ts, &vs, None, None)
    }

    /// One element of an array channel, windowed and decimated exactly as
    /// [`WasmMf4File::signal_window`] would treat a scalar channel:
    /// `{timestamps: Float64Array, values: Float64Array, name, unit, elements}`
    /// — `elements` being the selectable element count (the shape's product,
    /// or the largest real sample for a dynamic-shape array).
    ///
    /// A sample without that element (possible only in a dynamic-shape array)
    /// yields `NaN`, which the drawing side gaps. Element selection is the
    /// honest minimal view of a channel with several values per sample: the
    /// per-sample table (plan 2.2) will read the same flat cache this slices.
    pub fn signal_element_window(
        &mut self,
        name: &str,
        element: usize,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<js_sys::Object, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            values,
            payload,
            ..
        } = self.decoded(name)?;
        let (starts, elements_per_sample) = match payload {
            Payload::Array {
                elements_per_sample,
            } => (None, *elements_per_sample),
            Payload::ArrayVarLen { starts } => (Some(starts.as_slice()), 0usize),
            other => {
                return Err(js_err(format!(
                    "channel '{}' decodes as {}, not an array; signal_element_window is for array channels",
                    name,
                    other.name()
                )))
            }
        };
        let elements = match starts {
            None => elements_per_sample,
            // A dynamic shape has no single count; the largest real sample is
            // what a selector can offer.
            Some(starts) => starts
                .windows(2)
                .map(|w| w[1].saturating_sub(w[0]))
                .max()
                .unwrap_or(0),
        };
        let elem = element_values(values, starts, elements_per_sample, element);
        let (ts, vs) = decimate_window(timestamps, &elem, t0, t1, max_points);
        series_object(name, unit, &ts, &vs, Some(elements), None)
    }

    /// Defines a computed channel (plan 4.2): `expr` may reference any file
    /// channel (or another computed channel) as `[Name]`, and supports
    /// `+ - * /`, parentheses, unary minus, `abs`, `sqrt`, `min`, `max`.
    ///
    /// The expression is parsed and every reference checked *now* — a typo
    /// becomes a thrown error here, not a channel that plots as gaps. The
    /// evaluation timeline is the first referenced channel's, in its own
    /// master's units; other references are sampled at the nearest index.
    /// Returns the parsed reference list as a JSON array.
    pub fn define_computed(&mut self, name: &str, expr: &str) -> Result<String, JsValue> {
        if name.is_empty() {
            return Err(js_err("the computed channel needs a name"));
        }
        if self.bus.iter().any(|b| b.name == name)
            || self.computed.iter().any(|(n, _)| n == name)
            || self.inner.channel_names().iter().any(|n| *n == name)
        {
            return Err(js_err(format!(
                "the name '{name}' is already a channel of this file"
            )));
        }
        let parsed = Parser::parse(expr).map_err(js_err)?;
        for reference in &parsed.refs {
            let known = self.inner.channel_names().iter().any(|n| n == reference)
                || self.bus.iter().any(|b| b.name == *reference)
                || self.computed.iter().any(|(n, _)| n == reference);
            if !known {
                return Err(js_err(format!(
                    "the expression references '{reference}', which this file does not have"
                )));
            }
        }
        let refs = parsed.refs.clone();
        self.computed.push((name.to_string(), parsed));
        let mut out = String::from("{\"refs\":[");
        for (i, r) in refs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_json_str_into(r, &mut out);
            out.push('"');
        }
        out.push_str("]}");
        Ok(out)
    }

    /// Removes a computed channel. Returns whether one was removed.
    pub fn remove_computed(&mut self, name: &str) -> bool {
        let before = self.computed.len();
        self.computed.retain(|(n, _)| n != name);
        self.computed.len() != before
    }

    /// Builds a computed channel's series: the first reference's timeline,
    /// every reference sampled at the nearest index, the expression evaluated
    /// per sample in Rust over raw (not decimated) arrays.
    fn decode_computed(&self, name: &str) -> Result<CachedSeries, JsValue> {
        // Clone the expression out to end the &mut borrow before recursion.
        let expr = self
            .computed
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, e)| e.clone())
            .ok_or_else(|| {
                js_err(Mf4Error::ChannelNotFound {
                    name: name.to_string(),
                })
            })?;
        let mut refs_ts: Vec<Vec<f64>> = Vec::with_capacity(expr.refs.len());
        let mut refs_vals: Vec<Vec<f64>> = Vec::with_capacity(expr.refs.len());
        for r in &expr.refs {
            let series = self.fetch_series(r)?;
            refs_ts.push(series.timestamps.clone());
            refs_vals.push(series.values.clone());
        }
        let anchor_ts = refs_ts[0].clone();
        let mut values = Vec::with_capacity(anchor_ts.len());
        for &t in &anchor_ts {
            let mut args: Vec<f64> = Vec::with_capacity(expr.refs.len());
            for (ri, rts) in refs_ts.iter().enumerate() {
                let idx = rts.partition_point(|&x| x < t);
                let idx = if idx >= rts.len() {
                    rts.len().saturating_sub(1)
                } else if idx > 0 && (t - rts[idx - 1]) <= (rts[idx] - t) {
                    idx - 1
                } else {
                    idx
                };
                args.push(*refs_vals[ri].get(idx).unwrap_or(&f64::NAN));
            }
            values.push(expr.eval(&expr.root, &|i| args.get(i).copied().unwrap_or(f64::NAN)));
        }
        Ok(CachedSeries {
            unit: String::new(),
            timestamps: anchor_ts,
            values,
            payload: Payload::Scalar,
        })
    }

    /// Decodes the file's CAN bus logs against a DBC database, adding every
    /// decoded signal to the channel list as `"<message>.<signal>"` (a second
    /// bus carrying the same message gets `"@<bus>"`). Replaces any previous
    /// attach; an empty result is a valid outcome for a DBC that matches no
    /// logged identifier.
    ///
    /// Returns a JSON summary `{"signals": N, "names": [...]}` so the viewer
    /// can say what changed. Decoded signals behave like file channels in
    /// every endpoint — windows, stats, CSV, the table — because they ARE
    /// cached series; a value-table signal ships its labels as text.
    ///
    /// MDF 3 files carry no CAN logs, so attaching is refused there.
    pub fn attach_dbc(&mut self, dbc: &[u8]) -> Result<String, JsValue> {
        let Inner::V4(file) = &self.inner else {
            return Err(js_err(
                "DBC attach needs an MDF 4 bus log; MDF 3 files have no CAN frames",
            ));
        };
        let database = CanDatabase::from_dbc(dbc).map_err(js_err)?;
        let decoded = file.decode_bus(&database).map_err(js_err)?;

        // Build the overlay. Names must be unique across the whole channel
        // list, so a message+name seen on more than one bus is suffixed.
        let mut dup_buses: Vec<(String, String, u8)> = Vec::new();
        for signal in decoded.iter() {
            dup_buses.push((
                signal.message.to_string(),
                signal.name.to_string(),
                signal.bus_channel,
            ));
        }
        let mut overlay: Vec<BusChannel> = Vec::new();
        for signal in decoded.iter() {
            let base = format!("{}.{}", signal.message, signal.name);
            let bus_hits = dup_buses
                .iter()
                .filter(|(m, n, _)| *m == signal.message && *n == signal.name)
                .count();
            let name = if bus_hits > 1 {
                format!("{} (bus {})", base, signal.bus_channel)
            } else {
                base
            };
            // A value table turns the physical values into labels: the text
            // path draws them as state bands and reads them out as words.
            let has_texts = (0..signal.values.len()).any(|i| signal.text_at(i).is_some());
            let payload = if has_texts {
                Payload::Text(
                    (0..signal.values.len())
                        .map(|i| signal.text_at(i).map(str::to_string))
                        .collect(),
                )
            } else {
                Payload::Scalar
            };
            overlay.push(BusChannel {
                name,
                message: signal.message.to_string(),
                series: CachedSeries {
                    unit: signal.unit.to_string(),
                    timestamps: signal.timestamps.clone(),
                    values: signal.values.clone(),
                    payload,
                },
            });
        }
        let count = overlay.len();
        let mut names = String::from("{\"signals\":");
        let _ = write!(names, "{count},\"names\":[");
        for (i, bus) in overlay.iter().enumerate() {
            if i > 0 {
                names.push(',');
            }
            names.push('"');
            escape_json_str_into(&bus.name, &mut names);
            names.push('"');
        }
        names.push_str("]}");
        self.bus = overlay;
        Ok(names)
    }

    /// Removes a previous DBC attach's channels. Naming only the inverse of
    /// [`WasmMf4File::attach_dbc`]; a fresh attach replaces on its own.
    pub fn detach_dbc(&mut self) -> usize {
        let n = self.bus.len();
        self.bus.clear();
        n
    }

    /// The file's bus-log channel groups, as a JSON array of
    /// `{"kind":"can"|"lin", "group": N, "frames": N}` — `group` is the
    /// index [`WasmMf4File::bus_frames_page`] pages by. Empty for a file
    /// with no bus logging (the ordinary measurement file).
    pub fn bus_groups(&self) -> Result<String, JsValue> {
        let Inner::V4(_) = &self.inner else {
            return Ok("[]".to_string());
        };
        let mut out = String::from("[");
        let mut push = |f: &Mf4File, kind: &str, group: usize, first: &mut bool| {
            let groups: Vec<_> = if kind == "can" {
                f.can_frame_groups()
            } else {
                f.lin_frame_groups()
            };
            let Some(cg) = groups.get(group) else {
                return;
            };
            if !*first {
                out.push(',');
            }
            *first = false;
            let frames = if kind == "can" {
                f.can_frames(cg).map(|fr| fr.len()).unwrap_or(0)
            } else {
                f.lin_frames(cg).map(|fr| fr.len()).unwrap_or(0)
            };
            let _ = write!(
                out,
                "{{\"kind\":\"{kind}\",\"group\":{group},\"frames\":{frames}}}"
            );
        };
        // Enumerate kinds separately: can groups first, then lin, each
        // indexed from zero — the page call takes the kind, not one merged
        // index, so no ambiguity between a file's CAN and LIN logs.
        let mut first = true;
        if let Inner::V4(f) = &self.inner {
            for group in 0..f.can_frame_groups().len() {
                push(f, "can", group, &mut first);
            }
            for group in 0..f.lin_frame_groups().len() {
                push(f, "lin", group, &mut first);
            }
        }
        out.push(']');
        Ok(out)
    }

    /// One page of logged bus frames — the frame panel's data path — as
    /// JSON: `{"total","start","count","rows":[…]}`, each row a
    /// `{"t","id","dlc","ext","dir","bus","data"}` object with the payload
    /// as hex. `ext` marks 29-bit CAN ids; `dir` is `"tx"`/`"rx"` where the
    /// log records a direction and `null` where it does not; LIN rows carry
    /// the 6-bit LIN id and `null` ext/dir.
    pub fn bus_frames_page(
        &self,
        kind: &str,
        group: usize,
        start: usize,
        count: usize,
    ) -> Result<String, JsValue> {
        /// One kind-homogeneous page of frames, already sliced.
        enum Page {
            Can {
                rows: Vec<(f64, u32, usize, Option<bool>, u8, String)>,
            },
            Lin {
                rows: Vec<(f64, u32, usize, u8, String)>,
            },
        }
        impl Page {
            fn len(&self) -> usize {
                match self {
                    Page::Can { rows } => rows.len(),
                    Page::Lin { rows } => rows.len(),
                }
            }
            fn write_row(&self, i: usize, out: &mut String) {
                out.push_str("{\"t\":");
                match self {
                    Page::Can { rows } => {
                        let Some((t, id, dlc, ext, bus, data)) = rows.get(i) else {
                            return;
                        };
                        write_f64(out, *t);
                        let _ = write!(
                            out,
                            ",\"id\":{id},\"dlc\":{dlc},\"ext\":{ext},\"dir\":null,\"bus\":{bus},\"data\":{data}}}",
                            ext = match ext {
                                Some(true) => "true",
                                Some(false) => "false",
                                None => "null",
                            },
                        );
                    }
                    Page::Lin { rows } => {
                        let Some((t, id, dlc, bus, data)) = rows.get(i) else {
                            return;
                        };
                        write_f64(out, *t);
                        let _ = write!(
                            out,
                            ",\"id\":{id},\"dlc\":{dlc},\"ext\":null,\"dir\":null,\"bus\":{bus},\"data\":{data}}}"
                        );
                    }
                }
            }
        }

        let Inner::V4(f) = &self.inner else {
            return Err(js_err("bus frames need an MDF 4 bus log"));
        };
        let slice = |total: usize, start: usize, count: usize| -> (usize, usize) {
            let s0 = start.min(total);
            (s0, (s0 + count).min(total))
        };
        // The group's full frame count — `total` in the reply, distinct from
        // the page's own row count.
        let frame_total: usize;
        let page = match kind {
            "can" => {
                let groups = f.can_frame_groups();
                let cg = groups
                    .get(group)
                    .ok_or_else(|| js_err(format!("no CAN channel group {group} in this file")))?;
                let frames = f.can_frames(cg).map_err(js_err)?;
                frame_total = frames.len();
                let (s0, e0) = slice(frames.len(), start, count);
                let mut rows = Vec::with_capacity(e0 - s0);
                for i in s0..e0 {
                    if let Some(frame) = frames.get(i) {
                        let mut data = String::with_capacity(frame.data.len() * 3 + 4);
                        hex_field(frame.data, &mut data);
                        rows.push((
                            frame.timestamp,
                            frame.id,
                            frame.data.len(),
                            frame.extended,
                            frame.bus_channel,
                            data,
                        ));
                    }
                }
                Page::Can { rows }
            }
            "lin" => {
                let groups = f.lin_frame_groups();
                let cg = groups
                    .get(group)
                    .ok_or_else(|| js_err(format!("no LIN channel group {group} in this file")))?;
                let frames = f.lin_frames(cg).map_err(js_err)?;
                frame_total = frames.len();
                let (s0, e0) = slice(frames.len(), start, count);
                let mut rows = Vec::with_capacity(e0 - s0);
                for i in s0..e0 {
                    if let Some(frame) = frames.get(i) {
                        let mut data = String::with_capacity(frame.data.len() * 3 + 4);
                        hex_field(frame.data, &mut data);
                        rows.push((
                            frame.timestamp,
                            frame.id as u32,
                            frame.data.len(),
                            frame.bus_channel,
                            data,
                        ));
                    }
                }
                Page::Lin { rows }
            }
            other => return Err(js_err(format!("unknown bus kind '{other}' (can or lin)"))),
        };

        let count = page.len();
        let mut out = String::with_capacity(64 + page.len() * 72);
        out.push_str("{\"total\":");
        let _ = write!(out, "{frame_total}");
        out.push_str(",\"start\":");
        let _ = write!(out, "{}", start.min(frame_total));
        out.push_str(",\"count\":");
        let _ = write!(out, "{count}");
        out.push_str(",\"rows\":[");
        for i in 0..count {
            if i > 0 {
                out.push(',');
            }
            page.write_row(i, &mut out);
        }
        out.push_str("]}");
        Ok(out)
    }

    /// The index of the frame nearest time `t` in a bus group — the frame
    /// panel's cursor link, so a plot cursor can be answered without
    /// shipping the whole frame timeline to the UI. Frames are time-sorted
    /// (a bus log is a recording), so this is a binary search over `get`.
    pub fn bus_frame_locate(&self, kind: &str, group: usize, t: f64) -> Result<usize, JsValue> {
        let Inner::V4(f) = &self.inner else {
            return Err(js_err("bus frames need an MDF 4 bus log"));
        };
        let len = match kind {
            "can" => {
                let groups = f.can_frame_groups();
                let cg = groups
                    .get(group)
                    .ok_or_else(|| js_err(format!("no CAN channel group {group} in this file")))?;
                f.can_frames(cg).map_err(js_err)?.len()
            }
            "lin" => {
                let groups = f.lin_frame_groups();
                let cg = groups
                    .get(group)
                    .ok_or_else(|| js_err(format!("no LIN channel group {group} in this file")))?;
                f.lin_frames(cg).map_err(js_err)?.len()
            }
            other => return Err(js_err(format!("unknown bus kind '{other}'"))),
        };
        if len == 0 {
            return Ok(0);
        }
        // Plain binary search over the frame timestamps via the kind's own
        // accessor; a frame not exactly at `t` resolves to its neighbour.
        let stamp = |i: usize| -> f64 {
            match kind {
                "can" => {
                    let groups = f.can_frame_groups();
                    let Some(cg) = groups.get(group) else {
                        return f64::NAN;
                    };
                    let Ok(frames) = f.can_frames(cg) else {
                        return f64::NAN;
                    };
                    frames.get(i).map(|fr| fr.timestamp).unwrap_or(f64::NAN)
                }
                _ => {
                    let groups = f.lin_frame_groups();
                    let Some(cg) = groups.get(group) else {
                        return f64::NAN;
                    };
                    let Ok(frames) = f.lin_frames(cg) else {
                        return f64::NAN;
                    };
                    frames.get(i).map(|fr| fr.timestamp).unwrap_or(f64::NAN)
                }
            }
        };
        let (mut lo, mut hi) = (0usize, len);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if stamp(mid) < t {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        // `lo` is the first frame at or after t; prefer the closer neighbour.
        if lo > 0 && (lo == len || (t - stamp(lo - 1)) <= (stamp(lo) - t)) {
            Ok(lo - 1)
        } else {
            Ok(lo)
        }
    }

    /// Detects a (latitude, longitude) channel pair by name, porting the
    /// GUI panel's heuristics: token/substring matching on the usual GPS
    /// names, with dynamics and errors disqualified. Returns
    /// `{"latitude": name|null, "longitude": name|null}` — `null` halves
    /// mean "not found", which the viewer answers by hiding the panel.
    pub fn detect_gps_channels(&self) -> Result<String, JsValue> {
        let names = self.inner.channel_names();
        let latitude = names.iter().find(|n| is_latitude_channel_name(n));
        let longitude = names.iter().find(|n| is_longitude_channel_name(n));
        let mut out = String::with_capacity(96);
        out.push_str("{\"latitude\":");
        match latitude {
            Some(n) => {
                out.push('"');
                escape_json_str_into(n, &mut out);
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"longitude\":");
        match longitude {
            Some(n) => {
                out.push('"');
                escape_json_str_into(n, &mut out);
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push('}');
        Ok(out)
    }

    /// A GPS track from a (latitude, longitude [, speed]) channel triple, as
    /// JSON `{"n":N, "t":[…], "lat":[…], "lon":[…], "speed":[…]|null,
    /// "aligned":bool}`. The latitude channel's timestamps anchor the track;
    /// `aligned` is false when longitude/speed carried fewer samples than
    /// latitude (their tails become null/gap — a track never invents points).
    /// A track longer than [`TRACK_MAX_POINTS`] is stride-decimated in Rust:
    /// the drawn polyline is what matters, and shipping millions of raw
    /// coordinates to a canvas would be the waste the other endpoints avoid.
    pub fn gps_track(
        &mut self,
        lat: &str,
        lon: &str,
        speed: Option<String>,
    ) -> Result<String, JsValue> {
        fn series_of(file: &mut WasmMf4File, name: &str) -> Result<(Vec<f64>, Vec<f64>), JsValue> {
            let series = file.decoded(name)?;
            Ok((series.timestamps.clone(), series.values.clone()))
        }
        let (t, lats) = series_of(self, lat)?;
        let (_lon_t, lons) = series_of(self, lon)?;
        let speed = match &speed {
            Some(name) => Some(series_of(self, name)?),
            None => None,
        };

        // Alignment: everything rides the latitude channel's clock. A
        // shorter longitude/speed is padded with NaN (gaps), a longer one is
        // truncated; `aligned` says whether the masters truly matched.
        // The latitude timeline anchors the track; shorter companions pad
        // with NaN so the track keeps its full time range with honest gaps.
        let n_full = t.len();
        let aligned = t.len() == lons.len()
            && speed
                .as_ref()
                .map(|(_, sp)| sp.len() == t.len())
                .unwrap_or(true);

        // Stride decimation for very long tracks (uniform, not min/max: a
        // track's shape survives point thinning; min/max is a time-axis tool).
        let stride = ((n_full as f64) / TRACK_MAX_POINTS).ceil().max(1.0) as usize;
        let mut out = String::with_capacity(48 + (n_full / stride) * 48);
        out.push_str("{\"n\":");
        let mut count = 0usize;
        let push_point = |out: &mut String, count: &mut usize| {
            if *count > 0 {
                out.push(',');
            }
            *count += 1;
        };
        out.push('[');
        let mut lat_out = String::new();
        let mut lon_out = String::new();
        let mut t_out = String::new();
        let mut sp_out = String::from("[");
        let mut sp_count = 0usize;
        let speed_vals = speed.as_ref().map(|(_, v)| v);
        for i in (0..n_full).step_by(stride) {
            let lat = lats[i];
            if !lat.is_finite() {
                continue; // a broken reading is a gap, not a point at 0,0
            }
            push_point(&mut out, &mut count);
            let _ = write!(lat_out, "{},", lats[i]);
            let lon = lons.get(i).copied().unwrap_or(f64::NAN);
            let _ = write!(
                lon_out,
                "{}",
                if lon.is_finite() {
                    lon.to_string()
                } else {
                    "null".to_string()
                }
            );
            lon_out.push(',');
            write_f64(&mut t_out, t[i]);
            t_out.push(',');
            if let Some((_, sp)) = &speed {
                match sp.get(i).copied() {
                    Some(v) if v.is_finite() => {
                        let _ = write!(sp_out, "{v}");
                    }
                    _ => sp_out.push_str("null"),
                }
                sp_out.push(',');
                sp_count += 1;
            }
        }
        out.push(']');
        // Assemble the final object with the per-array buffers.
        let mut full = String::with_capacity(
            out.len() + lat_out.len() + lon_out.len() + t_out.len() + sp_out.len() + 64,
        );
        full.push_str("{\"n\":");
        let _ = write!(full, "{count}");
        full.push_str(",\"t\":");
        full.push_str(&format!("[{}]", t_out.trim_end_matches(',')));
        full.push_str(",\"lat\":");
        full.push_str(&format!("[{}]", lat_out.trim_end_matches(',')));
        full.push_str(",\"lon\":");
        full.push_str(&format!("[{}]", lon_out.trim_end_matches(',')));
        full.push_str(",\"speed\":");
        match speed_vals {
            Some(_) => {
                full.push_str(&sp_out);
                if sp_count > 0 {
                    full.pop();
                }
                full.push(']');
            }
            None => full.push_str("null"),
        }
        full.push_str(",\"aligned\":");
        full.push_str(if aligned { "true" } else { "false" });
        full.push('}');
        Ok(full)
    }

    /// Raw per-sample payloads for an **array or bytes** channel, restricted
    /// to the index range `[start, start + count)`, as JSON — the sample
    /// table's data path for the kinds one numeric column cannot hold:
    ///
    /// - fixed-shape array: `{"kind":"array","eps":N,"elems":[flat…]}`
    /// - dynamic-shape array: `{"kind":"array","elems":[flat…],"starts":[…]}`
    ///   with page-local starts (one per row plus a final end)
    /// - bytes: `{"kind":"bytes","hex":["a1 2b …", …]}`, one hex string per
    ///   sample, ellipsized past 16 bytes
    ///
    /// plus the common `{name, start, total, count, times:[…]}` header. The
    /// scalar and text kinds have their own endpoints, so this refuses them:
    /// exactly one way to table each kind.
    ///
    /// Pure indexing over the decode cache — a range past the end clamps
    /// instead of panicking, and an empty page is a valid empty payload.
    pub fn raw_page(&mut self, name: &str, start: usize, count: usize) -> Result<String, JsValue> {
        let CachedSeries {
            timestamps,
            values,
            payload,
            ..
        } = self.decoded(name)?;
        let total = timestamps.len();
        let start = start.min(total);
        let end = (start + count).min(total);
        let mut out = String::with_capacity(64 + (end - start) * 24);
        out.push_str("{\"name\":\"");
        escape_json_str_into(name, &mut out);
        out.push_str("\",\"start\":");
        let _ = write!(out, "{start}");
        out.push_str(",\"total\":");
        let _ = write!(out, "{total}");
        out.push_str(",\"count\":");
        let _ = write!(out, "{}", end - start);
        out.push_str(",\"times\":[");
        for (i, &t) in timestamps[start..end].iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, t);
        }
        out.push_str("],");
        match payload {
            Payload::Array {
                elements_per_sample,
            } => {
                let eps = *elements_per_sample;
                out.push_str("\"kind\":\"array\",\"eps\":");
                let _ = write!(out, "{eps}");
                out.push_str(",\"elems\":[");
                for (i, &v) in values[start * eps..end * eps].iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_f64(&mut out, v);
                }
                out.push_str("]}");
            }
            Payload::ArrayVarLen { starts } => {
                out.push_str("\"kind\":\"array\",\"elems\":[");
                for (i, &v) in values[starts[start]..starts[end]].iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_f64(&mut out, v);
                }
                out.push_str("],\"starts\":[");
                for &s in &starts[start..=end] {
                    // Page-local: row i's elements are
                    // elems[page_starts[i]..page_starts[i+1]].
                    let _ = write!(out, "{},", s - starts[start]);
                }
                out.push(']');
                out.pop(); // trailing comma → valid JSON array
                out.push('}');
            }
            Payload::Bytes { data, width } => {
                out.push_str("\"kind\":\"bytes\",\"hex\":[");
                for i in start..end {
                    if i > start {
                        out.push(',');
                    }
                    let sample = data.get(i * width..(i + 1) * width).unwrap_or(&[]);
                    hex_field(sample, &mut out);
                }
                out.push_str("]}");
            }
            Payload::VarBytes { data, starts } => {
                out.push_str("\"kind\":\"bytes\",\"hex\":[");
                for i in start..end {
                    if i > start {
                        out.push(',');
                    }
                    let from = starts[i];
                    let to = starts.get(i + 1).copied().unwrap_or(from);
                    let sample = data.get(from..to).unwrap_or(&[]);
                    hex_field(sample, &mut out);
                }
                out.push_str("]}");
            }
            other => {
                return Err(js_err(format!(
                    "channel '{}' decodes as {}, which the raw page is not for; \
                     scalar and text channels page through their own endpoints",
                    name,
                    other.name()
                )))
            }
        }
        Ok(out)
    }

    /// One channel's samples within `[t0, t1]` as CSV (`timestamp,<name>`
    /// header, one row per sample, non-finite values as empty fields),
    /// formatted in Rust so a "Download CSV" of the visible window costs the
    /// main thread one string.
    ///
    /// A text channel exports its labels — an empty value column for a
    /// channel whose every sample is a word would be the CSV form of the NaN
    /// this API used to ship. An array or byte channel refuses: one column
    /// cannot honestly hold several values per sample (that export is the
    /// table view's job, plan 2.2).
    pub fn signal_csv(&mut self, name: &str, t0: f64, t1: f64) -> Result<String, JsValue> {
        let CachedSeries {
            timestamps,
            values,
            payload,
            ..
        } = self.decoded(name)?;
        let times = &timestamps;
        match payload {
            Payload::Text(labels) => {
                let (x0, x1) = (finite_or(t0, times.first()), finite_or(t1, times.last()));
                let (Some(x0), Some(x1)) = (x0, x1) else {
                    return Ok(labels_csv(&[], &[], name));
                };
                let start = times.partition_point(|&t| t < x0);
                let end = times.partition_point(|&t| t <= x1);
                let ls: Vec<Option<&str>> = labels
                    .get(start..end)
                    .unwrap_or(&[])
                    .iter()
                    .map(|l| l.as_deref())
                    .collect();
                Ok(labels_csv(times.get(start..end).unwrap_or(&[]), &ls, name))
            }
            Payload::Array { .. }
            | Payload::ArrayVarLen { .. }
            | Payload::Bytes { .. }
            | Payload::VarBytes { .. } => Err(js_err(format!(
                "channel '{}' decodes as {}, which one CSV column cannot hold; \
                     export an element or wait for the per-sample table view",
                name,
                payload.name()
            ))),
            // The scalar path predates the payload cache and read the series
            // fresh per call; the cache holds the same to_f64 + validity
            // fold, so the bytes it emits are unchanged.
            Payload::Scalar => {
                let (x0, x1) = (finite_or(t0, times.first()), finite_or(t1, times.last()));
                let (Some(x0), Some(x1)) = (x0, x1) else {
                    return Ok(series_csv(&[], &[], name));
                };
                let start = times.partition_point(|&t| t < x0);
                let end = times.partition_point(|&t| t <= x1);
                Ok(series_csv(
                    times.get(start..end).unwrap_or(&[]),
                    values.get(start..end).unwrap_or(&[]),
                    name,
                ))
            }
        }
    }

    /// Statistics over `name`'s samples inside `[t0, t1]` as JSON. A scalar
    /// channel gets `{count, invalid, min, max, mean, first, last, t0, t1}` —
    /// see [`window_stats_json`] for the exact window and NaN semantics (they
    /// mirror `signal_window`'s). A text channel gets the label distribution
    /// `{count, invalid, t0, t1, labels:[{label, samples, seconds}, …]}`
    /// ([`window_label_stats_json`]): min/max/mean of labels is not a thing,
    /// and how long each state was active is. `t0`/`t1` echo the window as
    /// applied, so a request made with infinite bounds can still label its
    /// figures.
    ///
    /// An array or byte channel is an error rather than a zero-count payload:
    /// it has no scalar series to describe, and a silent `count: 0` would read
    /// as "no data in this window", which is a different (wrong) statement.
    ///
    /// Serves both the region between the viewer's two cursors and the
    /// selected channel over the visible window — one call per channel per
    /// region change — off the same decode cache as `signal_window`.
    pub fn signal_stats(&mut self, name: &str, t0: f64, t1: f64) -> Result<String, JsValue> {
        let CachedSeries {
            timestamps,
            values,
            payload,
            ..
        } = self.decoded(name)?;
        match payload {
            Payload::Scalar => Ok(window_stats_json(timestamps, values, t0, t1)),
            Payload::Text(labels) => {
                let refs: Vec<Option<&str>> = labels.iter().map(|l| l.as_deref()).collect();
                Ok(window_label_stats_json(timestamps, &refs, t0, t1))
            }
            Payload::Array { .. }
            | Payload::ArrayVarLen { .. }
            | Payload::Bytes { .. }
            | Payload::VarBytes { .. } => Err(js_err(format!(
                "channel '{}' decodes as {}, which has no scalar statistics; \
                     ask an element of an array channel instead",
                name,
                payload.name()
            ))),
        }
    }

    /// The file's internal structure — the outline a viewer's structure panel
    /// draws, from the identification block down to a single channel — as one
    /// JSON document. Metadata only: nothing here decodes samples.
    ///
    /// Shape (v4): `{"format":4, "version", "block_count", "id_block",
    /// "hd_block", "history":[{"time","tool"}], "attachments":[{"name",
    /// "embedded","size"}], "events":[{"name","type","position"}],
    /// "hierarchy":[{"name","channels":[names…],"unresolved",n,"children":[…]}],
    /// "data_groups":[{"index","sorted","comment","channel_groups":[
    /// {"index","name","samples","bus","vlsd","comment",
    /// "reductions":[{"cycles","interval","sync"}],
    /// "channels":[{"index","name","unit","master","array","kind",
    /// "unreadable"}]}]}]}`. `kind` is the same string [`WasmMf4File::channels`]
    /// carries, so a viewer marks unplotable channels without a second call.
    ///
    /// MDF 3 has no block map, history, attachments, events or hierarchy (the
    /// format does not carry them); its groups and channels come in the same
    /// shapes, `master` marking the group's time channel and `block_count`
    /// absent. Non-finite `position`/`interval` values are `null`.
    pub fn structure(&self) -> Result<String, JsValue> {
        let mut out = String::with_capacity(4 * 1024);
        match &self.inner {
            Inner::V4(f) => {
                out.push_str("{\"format\":4,\"version\":\"");
                escape_json_str_into(&f.version().to_string(), &mut out);
                out.push('"');
                // The block walk never fails: a file too damaged to walk
                // yields few blocks and many warnings, which is the honest
                // answer for a viewer.
                let blocks = f.block_map();
                let _ = write!(out, ",\"block_count\":{}", blocks.blocks.len());
                // The two blocks at fixed addresses are named here rather than
                // left to a block list: they are the file's front door, and a
                // structure tree is where a reader starts.
                if let Some(id) = blocks.block_at(0) {
                    out.push_str(",\"id_block\":\"");
                    escape_json_str_into(&id.block_type, &mut out);
                    out.push('"');
                }
                if let Some(hd) = blocks.block_at(64) {
                    out.push_str(",\"hd_block\":\"");
                    escape_json_str_into(&hd.block_type, &mut out);
                    out.push('"');
                }

                out.push_str(",\"history\":[");
                for (index, entry) in f.file_history().iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"time\":\"");
                    escape_json_str_into(&entry.time.to_iso8601(), &mut out);
                    out.push_str("\",\"tool\":\"");
                    let tool = [entry.tool_vendor(), entry.tool_id(), entry.tool_version()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    escape_json_str_into(&tool, &mut out);
                    out.push_str("\"}");
                }

                out.push_str("],\"attachments\":[");
                for (index, attachment) in f.attachments().iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"name\":\"");
                    escape_json_str_into(&attachment.file_name, &mut out);
                    let _ = write!(
                        out,
                        "\",\"embedded\":{},\"size\":{}",
                        attachment.is_embedded, attachment.original_size
                    );
                    out.push('}');
                }

                out.push_str("],\"events\":[");
                for (index, event) in f.events().iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"name\":\"");
                    escape_json_str_into(&event.name, &mut out);
                    out.push_str("\",\"type\":\"");
                    // The event type's Debug spelling is the name a viewer
                    // falls back to when the file declares none — the same
                    // string the GUI's tree shows.
                    escape_json_str_into(&format!("{:?}", event.event_type), &mut out);
                    out.push_str("\",\"position\":");
                    write_f64(&mut out, event.position());
                    out.push('}');
                }

                out.push_str("],\"hierarchy\":[");
                for (index, node) in f.channel_hierarchy().iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_hierarchy_node(f, node, &mut out);
                }

                out.push_str("],\"data_groups\":[");
                for (dg_index, dg) in f.data_groups().iter().enumerate() {
                    if dg_index > 0 {
                        out.push(',');
                    }
                    let _ = write!(
                        out,
                        "{{\"index\":{dg_index},\"sorted\":{}",
                        !dg.is_unsorted()
                    );
                    out.push_str(",\"comment\":\"");
                    escape_json_str_into(&dg.comment, &mut out);
                    out.push_str("\",\"channel_groups\":[");
                    for (cg_index, cg) in dg.channel_groups.iter().enumerate() {
                        if cg_index > 0 {
                            out.push(',');
                        }
                        write_channel_group(
                            &mut out,
                            cg_index,
                            cg.sample_count,
                            &cg.acquisition_name,
                            cg.is_bus_event(),
                            cg.is_vlsd(),
                            &cg.comment,
                            cg.sample_reductions(),
                        );
                        for (ch_index, ch) in cg.channels.iter().enumerate() {
                            if ch_index > 0 {
                                out.push(',');
                            }
                            write_channel_v4(&mut out, ch_index, ch);
                        }
                        out.push_str("]}");
                    }
                    out.push_str("]}");
                }
                out.push_str("]}");
            }
            Inner::V3(f) => {
                out.push_str("{\"format\":3,\"version\":\"");
                escape_json_str_into(f.version(), &mut out);
                out.push_str("\",\"history\":[],\"attachments\":[],\"events\":[],\"hierarchy\":[],\"data_groups\":[");
                for (dg_index, dg) in f.data_groups().iter().enumerate() {
                    if dg_index > 0 {
                        out.push(',');
                    }
                    // v3 groups share a record stream keyed by a one-byte id
                    // when several channel groups coexist; there is no
                    // unsorted-record index for the viewer to report.
                    let _ = write!(
                        out,
                        "{{\"index\":{dg_index},\"sorted\":true,\"comment\":\"\""
                    );
                    out.push_str(",\"channel_groups\":[");
                    for (cg_index, cg) in dg.channel_groups.iter().enumerate() {
                        if cg_index > 0 {
                            out.push(',');
                        }
                        // The v3 group's comment is the closest thing it has
                        // to an acquisition name, as in `channels()`.
                        write_channel_group(
                            &mut out,
                            cg_index,
                            cg.cycle_count as u64,
                            &cg.comment,
                            false,
                            false,
                            "",
                            &[],
                        );
                        for (ch_index, ch) in cg.channels.iter().enumerate() {
                            if ch_index > 0 {
                                out.push(',');
                            }
                            write_channel_v3(&mut out, ch_index, ch);
                        }
                        out.push_str("]}");
                    }
                    out.push_str("]}");
                }
                out.push_str("]}");
            }
        }
        Ok(out)
    }
}

/// One channel-group object of the structure tree, up to and including the
/// `"channels":[` opening — the caller writes the channel rows and closes the
/// brackets. `name` is the acquisition name (v4) or the group comment (v3,
/// the closest thing it has); `reductions` is empty for formats without
/// sample reduction.
// The two formats pass slightly different fields (v3 has no reductions), so
// the shared writer spells them out rather than taking both group types.
#[allow(clippy::too_many_arguments)]
fn write_channel_group(
    out: &mut String,
    index: usize,
    samples: u64,
    name: &str,
    bus: bool,
    vlsd: bool,
    comment: &str,
    reductions: &[falcon_mdf::model::SampleReduction],
) {
    out.push_str("{\"index\":");
    let _ = write!(out, "{index},\"samples\":{samples},\"name\":\"");
    escape_json_str_into(name, out);
    let _ = write!(out, "\",\"bus\":{bus},\"vlsd\":{vlsd},\"comment\":\"");
    escape_json_str_into(comment, out);
    out.push_str("\",\"reductions\":[");
    for (r_index, r) in reductions.iter().enumerate() {
        if r_index > 0 {
            out.push(',');
        }
        out.push_str("{\"cycles\":");
        let _ = write!(out, "{},\"interval\":", r.cycle_count);
        write_f64(out, r.interval);
        out.push_str(",\"sync\":\"");
        // The Debug spelling is what the GUI's tree prints for the sync
        // domain; a viewer formats the same string into its own row.
        escape_json_str_into(&format!("{:?}", r.sync_type), out);
        out.push('"');
        out.push('}');
    }
    out.push_str("],\"channels\":[");
}

/// One channel row of a v4 structure tree: identity, plot-relevant flags and
/// the kind string [`WasmMf4File::channels`] carries, so a viewer can mark
/// unplotable channels from this document alone.
fn write_channel_v4(out: &mut String, index: usize, ch: &Channel) {
    out.push_str("{\"index\":");
    let _ = write!(out, "{index},\"name\":\"");
    escape_json_str_into(&ch.name, out);
    out.push_str("\",\"unit\":\"");
    escape_json_str_into(&ch.unit, out);
    let _ = write!(
        out,
        "\",\"master\":{},\"array\":{},\"kind\":\"{}\"",
        ch.is_master(),
        ch.is_array(),
        kind_of_channel(ch)
    );
    out.push_str(",\"unreadable\":");
    match ch.unreadable() {
        Some(reason) => {
            out.push('"');
            escape_json_str_into(&reason.to_string(), out);
            out.push('"');
        }
        None => out.push_str("null"),
    }
    out.push('}');
}

/// The v3 channel row: the same shape minus array/unreadable (MDF 3 has
/// neither arrays nor this build's unreadable layouts), `master` marking the
/// group's time channel, and the kind from the data-type code exactly as
/// [`Inner::channel_kind`] maps it.
fn write_channel_v3(out: &mut String, index: usize, ch: &falcon_mdf::mdf3::Mdf3Channel) {
    out.push_str("{\"index\":");
    let _ = write!(out, "{index},\"name\":\"");
    escape_json_str_into(&ch.name, out);
    out.push_str("\",\"unit\":\"");
    escape_json_str_into(&ch.unit, out);
    let kind = match ch.data_type {
        7 => "text",
        8 => "bytes",
        _ => "f64",
    };
    let _ = write!(
        out,
        "\",\"master\":{},\"array\":false,\"kind\":\"{kind}\",\"unreadable\":null}}",
        ch.is_time()
    );
}

/// One hierarchy node and, recursively, its children. Elements resolve
/// through the reader like the GUI's tree does; ones that do not resolve are
/// counted in `unresolved` rather than silently dropped.
fn write_hierarchy_node(
    file: &Mf4File,
    node: &falcon_mdf::model::ChannelHierarchyNode,
    out: &mut String,
) {
    out.push_str("{\"name\":\"");
    escape_json_str_into(&node.name, out);
    let mut names: Vec<&str> = Vec::with_capacity(node.elements.len());
    let mut unresolved = 0usize;
    for element in &node.elements {
        match file.channel_at(element) {
            Some(channel) => names.push(&channel.name),
            None => unresolved += 1,
        }
    }
    out.push_str("\",\"channels\":[");
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        escape_json_str_into(name, out);
        out.push('"');
    }
    let _ = write!(out, "],\"unresolved\":{unresolved},\"children\":[");
    for child in &node.children {
        write_hierarchy_node(file, child, out);
        out.push(',');
    }
    if !node.children.is_empty() {
        out.pop();
    }
    out.push_str("]}");
}

/// `bound` when finite, otherwise the series extent `fallback` (an empty
/// series has none, which the caller turns into an empty CSV).
fn finite_or(bound: f64, fallback: Option<&f64>) -> Option<f64> {
    if bound.is_finite() {
        Some(bound)
    } else {
        fallback.copied()
    }
}

/// Decodes one v4 channel into the cache: values keep `to_f64()`'s view,
/// validity folds into it, and the payload carries what that view cannot.
fn decode_v4(file: &Mf4File, name: &str) -> Result<CachedSeries, JsValue> {
    let channel = file
        .find_channel(name)
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: name.to_string(),
        })
        .map_err(js_err)?;
    let unit = channel.unit.clone();
    let series = file.time_series(channel).map_err(js_err)?;
    let mut values = series.values.to_f64();
    fold_validity(&mut values, series.validity.as_deref());

    // The f64 view above stays what it always was (text and bytes decode
    // to NaN, arrays to their flat elements) so the scalar endpoints do
    // not change; the payload carries what that view cannot express.
    let payload = map_payload(&series.values, series.validity.as_deref());

    Ok(CachedSeries {
        unit,
        timestamps: series.timestamps.to_vec(),
        values,
        payload,
    })
}

/// Decodes one v3 channel into the same cache shape. MDF 3 has no
/// per-sample invalidation bits, and the reader applies conversions inside
/// `channel_physical`, so the physical values and the group's time channel
/// are everything a series needs. A group without a time channel falls back
/// to sample indices — the same "one tick per sample" reading every other
/// masterless MDF viewer falls back to.
fn decode_v3(file: &Mdf3File, name: &str) -> Result<CachedSeries, JsValue> {
    let mut location = None;
    for (g, dg) in file.data_groups().iter().enumerate() {
        for (c, cg) in dg.channel_groups.iter().enumerate() {
            if let Some(i) = cg.channels.iter().position(|ch| ch.name == name) {
                location = Some((g, c, i));
            }
        }
    }
    let (g, c, i) = location
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: name.to_string(),
        })
        .map_err(js_err)?;

    let channel = &file.data_groups()[g].channel_groups[c].channels[i];
    let unit = channel.unit.clone();

    let values = file.channel_physical(g, c, i).map_err(js_err)?;
    // MDF 3 has no invalidation bits: every sample is valid.
    let payload = map_payload(&values, None);
    let mut values = values.to_f64();

    // The master supplies the timestamps; its values are the same count as
    // every channel in the group shares.
    let master_index = file.data_groups()[g].channel_groups[c]
        .channels
        .iter()
        .position(|ch| ch.is_time());
    let timestamps = match master_index {
        Some(mi) => file.channel_physical(g, c, mi).map_err(js_err)?.to_f64(),
        // No master: index-as-seconds keeps the channel drawable rather
        // than lying with a made-up clock.
        None => (0..values.len()).map(|idx| idx as f64).collect(),
    };
    if timestamps.len() != values.len() {
        // A master whose count disagrees with its group's channels is a
        // broken file; a shorter series is better than shifted samples.
        values.truncate(timestamps.len());
    }

    Ok(CachedSeries {
        unit,
        timestamps,
        values,
        payload,
    })
}

/// The payload view of decoded values, shared by both formats: the scalar
/// endpoints stay on `to_f64()` (text and bytes decode to NaN, arrays to
/// their flat elements, exactly as before), and complex/CANopen kinds land
/// in the scalar view too — their to_f64 is all-NaN (drawn as gaps), and
/// the channel list already marks them from metadata.
fn map_payload(values: &SignalValues, validity: Option<&[bool]>) -> Payload {
    match values {
        SignalValues::Str(texts) => Payload::Text(
            texts
                .iter()
                .enumerate()
                .map(|(i, t)| valid_at(validity, i, texts.len()).then(|| t.clone()))
                .collect(),
        ),
        SignalValues::Array {
            elements_per_sample,
            ..
        } => Payload::Array {
            elements_per_sample: *elements_per_sample,
        },
        SignalValues::ArrayVarLen { starts, .. } => Payload::ArrayVarLen {
            starts: starts.clone(),
        },
        SignalValues::Bytes { data, width } => Payload::Bytes {
            data: data.clone(),
            width: *width,
        },
        SignalValues::VarBytes { data, starts } => Payload::VarBytes {
            data: data.clone(),
            starts: starts.clone(),
        },
        _ => Payload::Scalar,
    }
}

/// Optional per-shape fields [`WasmMf4File::signal_arrays`] attaches so the
/// worker's raw cache can serve element readouts of array channels without a
/// second decode: `eps` for a fixed shape, `starts` for a dynamic one.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum Shape {
    Eps(usize),
    Starts(Vec<usize>),
}

/// Builds the `{timestamps, values, name, unit}` plain object shared by
/// [`WasmMf4File::signal_arrays`] and [`WasmMf4File::signal_window`]; the
/// element window adds `elements`, the selectable element count, and an
/// array channel's raw arrays add `shape`.
///
/// The typed arrays are copied out of wasm memory (not views into it), so the
/// receiving worker can move their buffers to the main thread and they stay
/// valid whatever the module does next.
fn series_object(
    name: &str,
    unit: &str,
    timestamps: &[f64],
    values: &[f64],
    elements: Option<usize>,
    shape: Option<Shape>,
) -> Result<js_sys::Object, JsValue> {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        let obj = js_sys::Object::new();
        let ts = js_sys::Float64Array::new_from_slice(timestamps);
        let vs = js_sys::Float64Array::new_from_slice(values);
        let set = |key: &str, val: JsValue| -> Result<(), JsValue> {
            js_sys::Reflect::set(obj.as_ref(), &JsValue::from_str(key), &val).map(|_| ())
        };
        set("timestamps", ts.into())?;
        set("values", vs.into())?;
        set("name", JsValue::from_str(name))?;
        set("unit", JsValue::from_str(unit))?;
        if let Some(elements) = elements {
            set("elements", JsValue::from_f64(elements as f64))?;
        }
        match shape {
            Some(Shape::Eps(eps)) => set("eps", JsValue::from_f64(eps as f64))?,
            // u32-sized on wasm: an f64 array loses nothing, and the consumer
            // is plain JS indexing.
            Some(Shape::Starts(starts)) => {
                let s = js_sys::Float64Array::new_from_slice(
                    &starts.iter().map(|&i| i as f64).collect::<Vec<f64>>(),
                );
                set("starts", s.into())?;
            }
            None => {}
        }
        Ok(obj)
    }
    // Native builds have no JS runtime to build the object in; the logic is
    // covered by the decimate_window/series_csv tests and the browser demo.
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = (name, unit, timestamps, values, elements, shape);
        Err(JsValue::NULL)
    }
}
